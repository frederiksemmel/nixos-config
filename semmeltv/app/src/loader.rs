use crate::artwork::{CARD_HEIGHT, CARD_WIDTH};
use image::ImageReader;
use image::imageops::FilterType;
use slint::{Rgb8Pixel, SharedPixelBuffer};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

pub struct Job<T> {
    pub target: T,
    pub path: PathBuf,
}

pub type Pixels = SharedPixelBuffer<Rgb8Pixel>;

pub fn spawn<T: Send + 'static>(deliver: impl Fn(T, Pixels) + Send + 'static) -> Sender<Job<T>> {
    let (sender, receiver) = mpsc::channel::<Job<T>>();
    thread::spawn(move || {
        for job in receiver {
            match decode(&job.path) {
                Ok(pixels) => deliver(job.target, pixels),
                Err(image::ImageError::IoError(error)) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => eprintln!("{}: {error}", job.path.display()),
            }
        }
    });
    sender
}

fn decode(path: &PathBuf) -> Result<Pixels, image::ImageError> {
    let image = ImageReader::open(path)?.with_guessed_format()?.decode()?;
    let image = if image.width() > CARD_WIDTH || image.height() > CARD_HEIGHT {
        image.resize_to_fill(CARD_WIDTH, CARD_HEIGHT, FilterType::Triangle)
    } else {
        image
    }
    .into_rgb8();
    Ok(SharedPixelBuffer::clone_from_slice(
        image.as_raw(),
        image.width(),
        image.height(),
    ))
}
