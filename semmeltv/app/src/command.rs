use slint::platform::Key;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Up,
    Down,
    Left,
    Right,
    Select,
    Back,
    PlayPause,
    Listen,
    Subtitles,
}

impl Command {
    pub fn from_key(text: &str) -> Option<Self> {
        let is = |key: Key| text == slint::SharedString::from(key).as_str();
        if is(Key::UpArrow) {
            Some(Self::Up)
        } else if is(Key::DownArrow) {
            Some(Self::Down)
        } else if is(Key::LeftArrow) {
            Some(Self::Left)
        } else if is(Key::RightArrow) {
            Some(Self::Right)
        } else if is(Key::Return) {
            Some(Self::Select)
        } else if is(Key::Escape) {
            Some(Self::Back)
        } else if text == " " {
            Some(Self::PlayPause)
        } else if text == "v" {
            Some(Self::Listen)
        } else if text == "s" {
            Some(Self::Subtitles)
        } else {
            None
        }
    }
}
