// tvcast casts local files to a DLNA renderer through go2tv's library, and
// takes playback commands over a unix socket while the cast runs.
//
//	tvcast list                                      renderers as "URL<TAB>name"
//	tvcast play -t URL [-s SUBS] [-start SECS] FILE  cast until the TV stops
//	tvcast load FILE                                 switch the running cast to FILE
//	tvcast toggle | seek SECS|+SECS|-SECS | position | stop
//
// play prints the TV's last known position in seconds when the cast ends.
// stop returns once the cast has ended. Commands for a running cast exit with
// status 3 when no cast is running.
package main

import (
	"bufio"
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"net"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"

	"go2tv.app/go2tv/v2/devices"
	"go2tv.app/go2tv/v2/httphandlers"
	"go2tv.app/go2tv/v2/soapcalls"
	"go2tv.app/go2tv/v2/utils"
)

var (
	errNoCast  = errors.New("no cast running")
	errStopped = errors.New("the cast stopped")
)

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: tvcast list | play | load | toggle | seek | position | stop")
		os.Exit(2)
	}
	var err error
	switch os.Args[1] {
	case "list":
		err = list()
	case "play":
		err = play(os.Args[2:])
	default:
		err = control(os.Args[1:])
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "tvcast:", err)
		if errors.Is(err, errNoCast) {
			os.Exit(3)
		}
		os.Exit(1)
	}
}

func socketPath() string {
	dir := os.Getenv("XDG_RUNTIME_DIR")
	if dir == "" {
		dir = os.TempDir()
	}
	return filepath.Join(dir, "tvcast.sock")
}

func list() error {
	devices.StartDiscovery(context.Background())
	time.Sleep(3 * time.Second)
	found, err := devices.LoadAllDevices()
	if errors.Is(err, devices.ErrNoDeviceAvailable) {
		return nil
	}
	if err != nil {
		return err
	}
	for _, d := range found {
		if d.Type == devices.DeviceTypeDLNA && !d.IsAudioOnly {
			fmt.Printf("%s\t%s\n", d.Addr, d.Name)
		}
	}
	return nil
}

// cast is a running cast to one renderer. mu serialises everything that
// talks to the TV, since load swaps tv and srv. As the httphandlers.Screen,
// its Fini fires when the TV reports that playback stopped.
type cast struct {
	target  string
	mu      sync.Mutex
	tv      *soapcalls.TVPayload
	srv     *httphandlers.HTTPserver
	last    int // last position the TV reported, -1 before the first
	stopped chan struct{}
	once    sync.Once
}

func (c *cast) EmitMsg(string)      {}
func (c *cast) SetMediaType(string) {}
func (c *cast) Fini()               { c.once.Do(func() { close(c.stopped) }) }

// load serves media and starts it on the TV, ending whatever played before.
func (c *cast) load(media, subs string) error {
	mtype, err := utils.GetMimeDetailsFromPath(media)
	if err != nil {
		return err
	}
	if c.tv != nil {
		_ = c.tv.SendtoTV("Stop")
		c.srv.StopServer()
	}

	tv, err := soapcalls.NewTVPayload(&soapcalls.Options{
		DMR:   c.target,
		Media: media,
		Subs:  subs,
		Mtype: mtype,
		Seek:  true,
	})
	if err != nil {
		return err
	}
	if subs == "" {
		tv.SubtitlesURL = ""
	}
	srv := httphandlers.NewServer(tv.ListenAddress())
	started := make(chan error, 1)
	go srv.StartServer(started, media, subs, tv, c)
	if err := <-started; err != nil {
		return err
	}
	if err := tv.SendtoTV("Play1"); err != nil {
		srv.StopServer()
		return err
	}
	c.tv, c.srv, c.last = tv, srv, -1
	return nil
}

func (c *cast) position() (int, error) {
	info, err := c.tv.GetPositionInfo()
	if err != nil {
		return 0, err
	}
	pos, err := utils.ClockTimeToSeconds(info[1])
	if err == nil {
		c.last = pos
	}
	return pos, err
}

// seek retries while the renderer refuses with UPnP error 701, which it does
// until playback has started and while a previous seek is still settling.
func (c *cast) seek(secs int) error {
	for range 50 {
		err := c.tv.SeekSoapCall(utils.SecondsToClockTime(secs))
		if err == nil || !strings.Contains(err.Error(), "<errorCode>701</errorCode>") {
			return err
		}
		select {
		case <-c.stopped:
			return errStopped
		case <-time.After(200 * time.Millisecond):
		}
	}
	return errors.New("the TV kept refusing to seek")
}

func play(args []string) error {
	fs := flag.NewFlagSet("play", flag.ExitOnError)
	target := fs.String("t", "", "renderer description URL, as printed by list")
	subs := fs.String("s", "", "subtitles file (.srt or .vtt)")
	start := fs.Int("start", 0, "start position in seconds")
	_ = fs.Parse(args)
	if *target == "" || fs.NArg() != 1 {
		return errors.New("usage: tvcast play -t URL [-s SUBS] [-start SECS] FILE")
	}

	media, err := filepath.Abs(fs.Arg(0))
	if err != nil {
		return err
	}
	if *subs != "" {
		if *subs, err = filepath.Abs(*subs); err != nil {
			return err
		}
	}

	ln, err := listen()
	if err != nil {
		return err
	}
	defer ln.Close()

	c := &cast{target: *target, stopped: make(chan struct{})}
	if err := c.load(media, *subs); err != nil {
		return err
	}
	defer func() { c.srv.StopServer() }()
	go serve(ln, c)
	go func() {
		c.mu.Lock()
		defer c.mu.Unlock()
		if *start > 0 {
			_ = c.seek(*start)
		}
	}()
	go func() {
		for {
			select {
			case <-c.stopped:
				return
			case <-time.After(time.Second):
			}
			c.mu.Lock()
			_, _ = c.position()
			c.mu.Unlock()
		}
	}()

	sig, cancel := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer cancel()
	select {
	case <-c.stopped:
	case <-sig.Done():
		c.mu.Lock()
		_ = c.tv.SendtoTV("Stop")
		c.mu.Unlock()
		c.Fini()
	}

	c.mu.Lock()
	defer c.mu.Unlock()
	if c.last >= 0 {
		fmt.Println(c.last)
	}
	return nil
}

// listen claims the control socket, refusing to take over one that a running
// cast still answers on.
func listen() (net.Listener, error) {
	sock := socketPath()
	if conn, err := net.Dial("unix", sock); err == nil {
		conn.Close()
		return nil, errors.New("another cast is already running")
	}
	_ = os.Remove(sock)
	return net.Listen("unix", sock)
}

func serve(ln net.Listener, c *cast) {
	for {
		conn, err := ln.Accept()
		if err != nil {
			return
		}
		go func() {
			line, err := bufio.NewReader(conn).ReadString('\n')
			if err != nil {
				conn.Close()
				return
			}
			name, arg, _ := strings.Cut(strings.TrimSuffix(line, "\n"), " ")
			c.mu.Lock()
			reply, err := handle(c, name, arg)
			c.mu.Unlock()
			if err != nil {
				reply = "error: " + err.Error()
			}
			fmt.Fprintln(conn, reply)
			if name == "stop" {
				// The connection closes when this process exits, after the
				// socket is gone, which is what `tvcast stop` waits for.
				c.Fini()
				return
			}
			conn.Close()
		}()
	}
}

func handle(c *cast, name, arg string) (string, error) {
	switch name {
	case "load":
		if err := c.load(arg, ""); err != nil {
			c.Fini()
			return "", err
		}
		return "loaded", nil
	case "toggle":
		info, err := c.tv.GetTransportInfo()
		if err != nil {
			return "", err
		}
		if info[0] == "PAUSED_PLAYBACK" {
			return "playing", c.tv.SendtoTV("Play")
		}
		return "paused", c.tv.SendtoTV("Pause")
	case "seek":
		secs, err := strconv.Atoi(arg)
		if err != nil {
			return "", errors.New("usage: seek SECS|+SECS|-SECS")
		}
		if strings.HasPrefix(arg, "+") || strings.HasPrefix(arg, "-") {
			pos, err := c.position()
			if err != nil {
				return "", err
			}
			secs += pos
		}
		secs = max(secs, 0)
		return utils.SecondsToClockTime(secs), c.seek(secs)
	case "position":
		pos, err := c.position()
		return strconv.Itoa(pos), err
	case "stop":
		_, _ = c.position()
		return "stopped", c.tv.SendtoTV("Stop")
	}
	return "", fmt.Errorf("unknown command %q", name)
}

func control(args []string) error {
	if args[0] == "load" {
		if len(args) != 2 {
			return errors.New("usage: tvcast load FILE")
		}
		abs, err := filepath.Abs(args[1])
		if err != nil {
			return err
		}
		args[1] = abs
	}
	conn, err := net.Dial("unix", socketPath())
	if err != nil {
		return errNoCast
	}
	defer conn.Close()
	if _, err := fmt.Fprintln(conn, strings.Join(args, " ")); err != nil {
		return err
	}
	r := bufio.NewReader(conn)
	reply, err := r.ReadString('\n')
	if err != nil {
		return err
	}
	_, _ = io.Copy(io.Discard, r)
	reply = strings.TrimSpace(reply)
	if msg, ok := strings.CutPrefix(reply, "error: "); ok {
		return errors.New(msg)
	}
	fmt.Println(reply)
	return nil
}
