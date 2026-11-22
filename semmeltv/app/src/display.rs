use serde_json::Value;
use std::fmt;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

const SETTLE: Duration = Duration::from_secs(2);
const ANSWER_WITHIN: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);
const TOLERANCE: f64 = 0.005;
const MAX_MULTIPLE: u32 = 5;
const UI_SIZE: (u64, u64) = (1920, 1080);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Video {
    pub width: u64,
    pub height: u64,
    pub fps: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mode {
    output: String,
    width: u64,
    height: u64,
    refresh: f64,
}

impl Mode {
    fn apply(&self) -> bool {
        let mode = format!("{}x{}@{:.6}Hz", self.width, self.height, self.refresh);
        answer("wlr-randr", &["--output", &self.output, "--mode", &mode], ANSWER_WITHIN).is_some()
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {}x{} @ {:.3} Hz",
            self.output, self.width, self.height, self.refresh
        )
    }
}

fn mismatch(refresh: f64, fps: f64) -> f64 {
    (1..=MAX_MULTIPLE)
        .map(|multiple| {
            let target = fps * f64::from(multiple);
            (refresh - target).abs() / target
        })
        .fold(f64::INFINITY, f64::min)
}

fn modes_of(output: &Value) -> Vec<(u64, u64, f64, bool)> {
    let Some(modes) = output["modes"].as_array() else {
        return Vec::new();
    };
    modes
        .iter()
        .filter_map(|mode| {
            Some((
                mode["width"].as_u64()?,
                mode["height"].as_u64()?,
                mode["refresh"].as_f64()?,
                mode["current"].as_bool().unwrap_or(false),
            ))
        })
        .collect()
}

struct Output {
    name: String,
    modes: Vec<(u64, u64, f64, bool)>,
}

impl Output {
    fn enabled(outputs: &Value) -> Option<Self> {
        let output = outputs
            .as_array()?
            .iter()
            .find(|output| output["enabled"].as_bool() == Some(true))?;
        Some(Self {
            name: output["name"].as_str()?.to_string(),
            modes: modes_of(output),
        })
    }

    fn change_to(&self, (width, height, refresh): (u64, u64, f64)) -> Option<Mode> {
        let current = self.modes.iter().find(|mode| mode.3)?;
        if (width, height, refresh) == (current.0, current.1, current.2) {
            return None;
        }
        Some(Mode {
            output: self.name.clone(),
            width,
            height,
            refresh,
        })
    }
}

fn choose(outputs: &Value, video: Video) -> Option<Mode> {
    let output = Output::enabled(outputs)?;
    let modes = &output.modes;
    let area = |mode: &(u64, u64, f64, bool)| mode.0 * mode.1;
    let covering: Vec<_> = modes
        .iter()
        .filter(|mode| mode.0 >= video.width && mode.1 >= video.height)
        .collect();
    let sizes = if covering.is_empty() {
        let largest = modes.iter().map(area).max()?;
        modes.iter().filter(|mode| area(mode) == largest).collect()
    } else {
        covering
    };
    let &&(width, height, refresh, _) = sizes
        .iter()
        .filter(|mode| mismatch(mode.2, video.fps) <= TOLERANCE)
        .min_by(|a, b| {
            area(a)
                .cmp(&area(b))
                .then(mismatch(a.2, video.fps).total_cmp(&mismatch(b.2, video.fps)))
                .then(a.2.total_cmp(&b.2))
        })?;
    output.change_to((width, height, refresh))
}

fn choose_ui(outputs: &Value) -> Option<Mode> {
    let output = Output::enabled(outputs)?;
    let &(width, height, refresh, _) = output
        .modes
        .iter()
        .filter(|mode| (mode.0, mode.1) == UI_SIZE)
        .max_by(|a, b| a.2.total_cmp(&b.2))?;
    output.change_to((width, height, refresh))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Wanted {
    film: Option<(u64, Video)>,
}

impl Wanted {
    const MENU: Self = Self { film: None };

    fn play(&mut self, player: u64, video: Video) {
        self.film = Some((player, video));
    }

    fn stop(&mut self, player: u64) {
        if self.film.is_some_and(|(playing, _)| playing == player) {
            self.film = None;
        }
    }

    fn change(&self, outputs: &Value) -> Option<Mode> {
        match self.film {
            Some((_, video)) => choose(outputs, video),
            None => choose_ui(outputs),
        }
    }

    fn purpose(&self) -> String {
        match self.film {
            Some((_, video)) => format!("a {}x{} video at {} fps", video.width, video.height, video.fps),
            None => "the menu".to_string(),
        }
    }
}

type Then = Box<dyn FnOnce() + Send>;

enum Change {
    Play(u64, Video),
    Stop(u64),
    Refresh,
}

static SETTLER: LazyLock<Sender<(Change, Option<Then>)>> = LazyLock::new(|| {
    let (settler, changes) = mpsc::channel::<(Change, Option<Then>)>();
    thread::spawn(move || {
        let mut wanted = Wanted::MENU;
        for (change, then) in changes {
            match change {
                Change::Play(player, video) => wanted.play(player, video),
                Change::Stop(player) => wanted.stop(player),
                Change::Refresh => {}
            }
            settle(&wanted);
            if let Some(then) = then {
                then();
            }
        }
    });
    settler
});

/// Runs `program` and gives its output, or nothing when it fails or has not finished `within`.
fn answer(program: &str, args: &[&str], within: Duration) -> Option<Vec<u8>> {
    let shown = format!("{program} {}", args.join(" "));
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!("display: {shown}: {error}");
            return None;
        }
    };
    let mut stdout = child.stdout.take()?;
    let output = thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.read_to_end(&mut output);
        output
    });
    let deadline = Instant::now() + within;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return output.join().ok(),
            Ok(Some(status)) => {
                eprintln!("display: {shown}: {status}");
                return None;
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("display: {shown}: no answer within {} s", within.as_secs());
                return None;
            }
        }
    }
}

fn outputs() -> Option<Value> {
    serde_json::from_slice(&answer("wlr-randr", &["--json"], ANSWER_WITHIN)?).ok()
}

fn settle(wanted: &Wanted) {
    if let Some(mode) = outputs().and_then(|outputs| wanted.change(&outputs)) {
        eprintln!("display: {mode} for {}", wanted.purpose());
        if mode.apply() {
            thread::sleep(SETTLE);
        }
    }
}

/// Asks for the mode that suits `video`, and calls `then` once the display shows it.
pub fn play(player: u64, video: Video, then: impl FnOnce() + Send + 'static) {
    let _ = SETTLER.send((Change::Play(player, video), Some(Box::new(then))));
}

pub fn stop(player: u64) {
    let _ = SETTLER.send((Change::Stop(player), None));
}

pub fn refresh() {
    let _ = SETTLER.send((Change::Refresh, None));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tv() -> Value {
        json!([{
            "name": "HDMI-A-1",
            "enabled": true,
            "modes": [
                { "width": 3840, "height": 2160, "refresh": 30.0, "current": true },
                { "width": 3840, "height": 2160, "refresh": 29.969999, "current": false },
                { "width": 3840, "height": 2160, "refresh": 25.0, "current": false },
                { "width": 3840, "height": 2160, "refresh": 24.0, "current": false },
                { "width": 3840, "height": 2160, "refresh": 23.976, "current": false },
                { "width": 1920, "height": 1080, "refresh": 60.0, "current": false },
                { "width": 1920, "height": 1080, "refresh": 50.0, "current": false },
                { "width": 1920, "height": 1080, "refresh": 30.0, "current": false },
                { "width": 1920, "height": 1080, "refresh": 25.0, "current": false },
                { "width": 1920, "height": 1080, "refresh": 24.0, "current": false },
                { "width": 1920, "height": 1080, "refresh": 23.976, "current": false },
                { "width": 1280, "height": 720, "refresh": 60.0, "current": false },
            ],
        }])
    }

    fn video(width: u64, height: u64, fps: f64) -> Video {
        Video { width, height, fps }
    }

    #[test]
    fn each_video_gets_its_display_mode() {
        let cases = [
            ("1080p film", video(1920, 1080, 23.976024), Some((1920, 1080, 23.976))),
            ("letterboxed film", video(1920, 800, 24.0), Some((1920, 1080, 24.0))),
            ("4k film", video(3840, 2160, 23.976), Some((3840, 2160, 23.976))),
            (
                "larger than every mode",
                video(7680, 4320, 24.0),
                Some((3840, 2160, 24.0)),
            ),
            (
                "smaller, rate only at 1080p",
                video(1280, 720, 25.0),
                Some((1920, 1080, 25.0)),
            ),
            (
                "smaller, rate at its own size",
                video(1280, 720, 60.0),
                Some((1280, 720, 60.0)),
            ),
            (
                "slightly off its nominal rate",
                video(1920, 1080, 23.944723),
                Some((1920, 1080, 23.976)),
            ),
            (
                "exact 24 beats 23.976",
                video(3840, 2160, 24.0),
                Some((3840, 2160, 24.0)),
            ),
            (
                "exact 29.97 beats 30",
                video(3840, 2160, 29.97003),
                Some((3840, 2160, 29.969999)),
            ),
            ("the current mode", video(3840, 2160, 30.0), None),
            ("a multiple of the current rate", video(3840, 2160, 15.0), None),
            ("a rate the display lacks", video(3840, 2160, 50.0), None),
            ("a rate nothing comes near", video(1920, 1080, 1000.0), None),
        ];
        for (name, video, want) in cases {
            let got = choose(&tv(), video).map(|mode| (mode.width, mode.height, mode.refresh));
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn the_ui_gets_1080p_at_its_fastest_rate() {
        let mut ui_set = tv();
        for mode in ui_set[0]["modes"].as_array_mut().unwrap() {
            let ui = mode["width"] == 1920 && mode["refresh"] == 60.0;
            mode["current"] = json!(ui);
        }
        let no_1080p = json!([{
            "name": "HDMI-A-1",
            "enabled": true,
            "modes": [{ "width": 1280, "height": 720, "refresh": 60.0, "current": true }],
        }]);
        let cases = [
            ("4k now", tv(), Some((1920, 1080, 60.0))),
            ("the ui mode already set", ui_set, None),
            ("a display without 1080p", no_1080p, None),
        ];
        for (name, outputs, want) in cases {
            let got = choose_ui(&outputs).map(|mode| (mode.width, mode.height, mode.refresh));
            assert_eq!(got, want, "{name}");
        }
    }

    fn showing(width: u64, refresh: f64) -> Value {
        let mut outputs = tv();
        for mode in outputs[0]["modes"].as_array_mut().unwrap() {
            mode["current"] = json!(mode["width"] == width && mode["refresh"] == refresh);
        }
        outputs
    }

    #[test]
    fn the_menu_always_gets_1080p60_back() {
        let cases = [
            (
                "after a 1080p 23.976 film",
                showing(1920, 23.976),
                Some((1920, 1080, 60.0)),
            ),
            ("after a 1080p 25 film", showing(1920, 25.0), Some((1920, 1080, 60.0))),
            ("after a 4k film", showing(3840, 23.976), Some((1920, 1080, 60.0))),
            ("after a 720p film", showing(1280, 60.0), Some((1920, 1080, 60.0))),
            ("in the menu's own mode", showing(1920, 60.0), None),
        ];
        for (name, outputs, want) in cases {
            let got = Wanted::MENU
                .change(&outputs)
                .map(|mode| (mode.width, mode.height, mode.refresh));
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn a_program_that_hangs_is_given_up_on_and_one_that_answers_is_heard() {
        let started = Instant::now();
        assert_eq!(answer("sleep", &["10"], Duration::from_millis(200)), None);
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        assert_eq!(answer("echo", &["mode"], ANSWER_WITHIN), Some(b"mode\n".to_vec()));
        assert_eq!(answer("false", &[], ANSWER_WITHIN), None);
    }

    #[test]
    fn a_film_keeps_its_mode_until_its_own_player_stops() {
        let got = video(1920, 1080, 23.976024);
        let mode = |wanted: &Wanted, outputs: &Value| {
            wanted
                .change(outputs)
                .map(|mode| (mode.width, mode.height, mode.refresh))
        };
        let mut wanted = Wanted::MENU;
        wanted.play(1, got);
        assert_eq!(
            mode(&wanted, &showing(1920, 60.0)),
            Some((1920, 1080, 23.976)),
            "playing"
        );
        assert_eq!(
            mode(&wanted, &showing(3840, 30.0)),
            Some((1920, 1080, 23.976)),
            "after the TV reset its mode"
        );
        wanted.play(2, got);
        wanted.stop(1);
        assert_eq!(
            mode(&wanted, &showing(1920, 23.976)),
            None,
            "the old player ending after the next one started"
        );
        wanted.stop(2);
        assert_eq!(
            mode(&wanted, &showing(1920, 23.976)),
            Some((1920, 1080, 60.0)),
            "the last player ending"
        );
    }
}
