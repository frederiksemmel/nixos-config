package main

import (
	"bufio"
	"crypto/tls"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"go2tv.app/go2tv/v2/soapcalls"
	"golang.org/x/net/websocket"
)

// knownTV is a renderer that list has found before, remembered so that it
// can be woken while it is off and reached without a discovery.
type knownTV struct {
	url, name, mac string
}

func (k knownTV) host() string {
	u, err := url.Parse(k.url)
	if err != nil {
		return ""
	}
	return u.Hostname()
}

func stateDir() string {
	dir := os.Getenv("XDG_STATE_HOME")
	if dir == "" {
		home, _ := os.UserHomeDir()
		dir = filepath.Join(home, ".local", "state")
	}
	return filepath.Join(dir, "tvcast")
}

func knownPath() string { return filepath.Join(stateDir(), "tvs") }

// loadKnown returns the remembered TVs, most recently found first.
func loadKnown() []knownTV {
	data, err := os.ReadFile(knownPath())
	if err != nil {
		return nil
	}
	var tvs []knownTV
	for line := range strings.Lines(string(data)) {
		f := strings.Split(strings.TrimSuffix(line, "\n"), "\t")
		if len(f) == 3 {
			tvs = append(tvs, knownTV{f[0], f[1], f[2]})
		}
	}
	return tvs
}

// remember records found TVs ahead of the remembered ones, replacing entries
// with the same address or MAC.
func remember(found []knownTV) error {
	if len(found) == 0 {
		return nil
	}
	tvs := found
	for _, old := range loadKnown() {
		replaced := false
		for _, f := range found {
			replaced = replaced || f.url == old.url || f.mac == old.mac
		}
		if !replaced {
			tvs = append(tvs, old)
		}
	}
	var b strings.Builder
	for _, t := range tvs {
		fmt.Fprintf(&b, "%s\t%s\t%s\n", t.url, t.name, t.mac)
	}
	if err := os.MkdirAll(stateDir(), 0o700); err != nil {
		return err
	}
	return os.WriteFile(knownPath(), []byte(b.String()), 0o600)
}

// macOf looks host up in the kernel's ARP table, which holds it after
// discovery fetched the renderer's description.
func macOf(host string) string {
	f, err := os.Open("/proc/net/arp")
	if err != nil {
		return ""
	}
	defer f.Close()
	s := bufio.NewScanner(f)
	for s.Scan() {
		fields := strings.Fields(s.Text())
		if len(fields) >= 4 && fields[0] == host && fields[3] != "00:00:00:00:00:00" {
			return fields[3]
		}
	}
	return ""
}

// wake broadcasts a Wake-on-LAN magic packet for mac.
func wake(mac string) error {
	hw, err := net.ParseMAC(mac)
	if err != nil {
		return err
	}
	packet := make([]byte, 0, 102)
	packet = append(packet, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff)
	for range 16 {
		packet = append(packet, hw...)
	}
	conn, err := net.DialUDP("udp4", nil, &net.UDPAddr{IP: net.IPv4bcast, Port: 9})
	if err != nil {
		return err
	}
	defer conn.Close()
	_, err = conn.Write(packet)
	return err
}

func wakeUp(args []string) error {
	if len(args) != 1 {
		return errors.New("usage: tvcast wake URL")
	}
	tv := knownTV{url: args[0], name: args[0]}
	for _, k := range loadKnown() {
		if k.url == tv.url {
			tv = k
		}
	}

	client := &http.Client{Timeout: time.Second}
	// A TV that is booting serves its description before its player takes
	// commands, so it answers once AVTransport reports a state.
	answers := func() bool {
		resp, err := client.Get(tv.url)
		if err != nil {
			return false
		}
		resp.Body.Close()
		if resp.StatusCode != http.StatusOK {
			return false
		}
		p, err := soapcalls.NewTVPayload(&soapcalls.Options{DMR: tv.url})
		if err != nil {
			return false
		}
		info, err := p.GetTransportInfo()
		return err == nil && info[0] != ""
	}

	start := time.Now()
	for time.Since(start) < time.Minute {
		if answers() {
			fmt.Println(tv.url)
			return nil
		}
		if tv.mac == "" {
			return fmt.Errorf("%s does not answer", tv.name)
		}
		if err := wake(tv.mac); err != nil {
			return err
		}
		if time.Since(start) < 15*time.Second {
			time.Sleep(500 * time.Millisecond)
			continue
		}
		// By now the TV may be up at another address.
		found, err := discover()
		if err != nil {
			return err
		}
		for _, d := range found {
			k := knownTV{url: d.Addr, name: d.Name}
			if macOf(k.host()) == tv.mac {
				if err := remember([]knownTV{{k.url, k.name, tv.mac}}); err != nil {
					return err
				}
				tv.url = k.url
			}
		}
	}
	return fmt.Errorf("%s did not turn on", tv.name)
}

// samsungKey presses key on the remote of the Samsung TV at host. The first
// connection asks for permission on the TV; the token it grants is kept for
// the next ones.
func samsungKey(host, key string) error {
	tokenFile := filepath.Join(stateDir(), "samsung-token-"+host)
	token, _ := os.ReadFile(tokenFile)

	q := url.Values{}
	q.Set("name", base64.StdEncoding.EncodeToString([]byte("tvcast")))
	if len(token) > 0 {
		q.Set("token", strings.TrimSpace(string(token)))
	}
	cfg, err := websocket.NewConfig(
		"wss://"+net.JoinHostPort(host, "8002")+"/api/v2/channels/samsung.remote.control?"+q.Encode(),
		"https://"+host,
	)
	if err != nil {
		return err
	}
	cfg.TlsConfig = &tls.Config{InsecureSkipVerify: true}
	cfg.Dialer = &net.Dialer{Timeout: 3 * time.Second}
	ws, err := websocket.DialConfig(cfg)
	if err != nil {
		return fmt.Errorf("the TV is off or unreachable: %w", err)
	}
	defer ws.Close()

	// The TV holds the connect event back until its permission prompt is
	// answered.
	_ = ws.SetDeadline(time.Now().Add(30 * time.Second))
	var msg struct {
		Event string `json:"event"`
		Data  struct {
			Token string `json:"token"`
		} `json:"data"`
	}
	if err := websocket.JSON.Receive(ws, &msg); err != nil {
		return err
	}
	switch msg.Event {
	case "ms.channel.connect":
	case "ms.channel.unauthorized":
		return errors.New("the TV refused the remote; allow tvcast under Settings > General > External Device Manager > Device Connection Manager")
	default:
		return fmt.Errorf("unexpected reply from the TV: %s", msg.Event)
	}
	if msg.Data.Token != "" && msg.Data.Token != strings.TrimSpace(string(token)) {
		if err := os.MkdirAll(stateDir(), 0o700); err != nil {
			return err
		}
		if err := os.WriteFile(tokenFile, []byte(msg.Data.Token), 0o600); err != nil {
			return err
		}
	}

	press, _ := json.Marshal(map[string]any{
		"method": "ms.remote.control",
		"params": map[string]string{
			"Cmd":          "Click",
			"DataOfCmd":    key,
			"Option":       "false",
			"TypeOfRemote": "SendRemoteKey",
		},
	})
	if err := websocket.Message.Send(ws, string(press)); err != nil {
		return err
	}
	// Closing right after the send can drop the key.
	time.Sleep(300 * time.Millisecond)
	return nil
}

// lastHost is the address of the most recently found TV.
func lastHost() (string, error) {
	known := loadKnown()
	if len(known) == 0 {
		return "", errors.New("no TV known yet; cast to it once first")
	}
	return known[0].host(), nil
}

// remoteToggle pauses or resumes whatever the most recently found TV plays,
// alternating between the remote's pause and play keys since it has no key
// that toggles.
func remoteToggle() (string, error) {
	host, err := lastHost()
	if err != nil {
		return "", err
	}
	last := filepath.Join(os.TempDir(), "tvcast-remote-paused")
	if dir := os.Getenv("XDG_RUNTIME_DIR"); dir != "" {
		last = filepath.Join(dir, "tvcast-remote-paused")
	}
	if _, err := os.Stat(last); err == nil {
		if err := samsungKey(host, "KEY_PLAY"); err != nil {
			return "", err
		}
		_ = os.Remove(last)
		return "playing", nil
	}
	if err := samsungKey(host, "KEY_PAUSE"); err != nil {
		return "", err
	}
	_ = os.WriteFile(last, nil, 0o600)
	return "paused", nil
}
