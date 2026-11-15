use crate::artwork::{CARD_HEIGHT, CARD_WIDTH};
use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageReader, RgbImage};
use slint::{Rgb8Pixel, SharedPixelBuffer};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;

const BLUR_SHRINK: u32 = 40;
const BACKGROUND_LIGHT: f32 = 0.45;

pub struct Art {
    pub path: PathBuf,
    pub fallback: Option<PathBuf>,
}

pub struct Job<T> {
    pub target: T,
    pub art: Art,
}

pub type Pixels = SharedPixelBuffer<Rgb8Pixel>;

pub fn spawn<T: Send + 'static>(deliver: impl Fn(T, Pixels) + Send + 'static) -> Sender<Job<T>> {
    let (sender, receiver) = mpsc::channel::<Job<T>>();
    thread::spawn(move || {
        for job in receiver {
            match load(&job.art) {
                Ok(image) => deliver(
                    job.target,
                    SharedPixelBuffer::clone_from_slice(image.as_raw(), image.width(), image.height()),
                ),
                Err(image::ImageError::IoError(error)) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => eprintln!("{}: {error}", job.art.path.display()),
            }
        }
    });
    sender
}

fn open(path: &Path) -> Result<DynamicImage, image::ImageError> {
    ImageReader::open(path)?.with_guessed_format()?.decode()
}

fn missing(result: &Result<DynamicImage, image::ImageError>) -> bool {
    matches!(result, Err(image::ImageError::IoError(error)) if error.kind() == ErrorKind::NotFound)
}

fn load(art: &Art) -> Result<RgbImage, image::ImageError> {
    let image = open(&art.path);
    match &art.fallback {
        Some(fallback) if missing(&image) => open(fallback).map(|poster| framed(&poster)),
        _ => image.map(fill),
    }
}

fn fill(image: DynamicImage) -> RgbImage {
    if image.width() > CARD_WIDTH || image.height() > CARD_HEIGHT {
        image.resize_to_fill(CARD_WIDTH, CARD_HEIGHT, FilterType::Triangle)
    } else {
        image
    }
    .into_rgb8()
}

pub fn framed(poster: &DynamicImage) -> RgbImage {
    let mut card = poster
        .resize_to_fill(
            CARD_WIDTH / BLUR_SHRINK,
            CARD_HEIGHT / BLUR_SHRINK,
            FilterType::Triangle,
        )
        .resize_exact(CARD_WIDTH, CARD_HEIGHT, FilterType::Triangle)
        .into_rgb8();
    for pixel in card.pixels_mut() {
        for channel in &mut pixel.0 {
            *channel = (f32::from(*channel) * BACKGROUND_LIGHT) as u8;
        }
    }
    let front = poster.resize(CARD_WIDTH, CARD_HEIGHT, FilterType::Triangle).into_rgb8();
    let left = (CARD_WIDTH - front.width()) / 2;
    let top = (CARD_HEIGHT - front.height()) / 2;
    imageops::replace(&mut card, &front, i64::from(left), i64::from(top));
    card
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    struct Dir(PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn dir(name: &str) -> Dir {
        let path = std::env::temp_dir().join(format!("semmeltv-loader-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Dir(path)
    }

    fn poster() -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(342, 513, |_, y| {
            if y < 256 {
                Rgb([200, 40, 40])
            } else {
                Rgb([40, 40, 200])
            }
        }))
    }

    #[test]
    fn a_poster_stands_upright_in_the_middle_of_a_darker_blur_of_itself() {
        let card = framed(&poster());
        assert_eq!(card.dimensions(), (CARD_WIDTH, CARD_HEIGHT));
        let front = CARD_HEIGHT * 342 / 513;
        let left = (CARD_WIDTH - front) / 2;
        let inside = |x: u32, y: u32| card.get_pixel(x, y).0;
        let [red, _, blue] = inside(CARD_WIDTH / 2, CARD_HEIGHT / 4);
        assert!(red > 180 && blue < 60, "the poster's top half shows as it is");
        let [red, _, blue] = inside(CARD_WIDTH / 2, CARD_HEIGHT * 3 / 4);
        assert!(blue > 180 && red < 60, "the poster's bottom half shows as it is");
        for x in [2, left - 4, left + front + 4, CARD_WIDTH - 3] {
            let [red, green, blue] = inside(x, CARD_HEIGHT / 4);
            assert!(
                red < 110 && green < 40 && blue < 110,
                "the sides at x {x} are a dim blur"
            );
        }
    }

    #[test]
    fn a_missing_backdrop_falls_back_to_the_poster_and_without_one_stays_missing() {
        let dir = dir("fallback");
        let backdrop = dir.0.join("backdrop.jpg");
        let poster_file = dir.0.join("poster.jpg");
        poster().save(&poster_file).unwrap();
        let art = |fallback: Option<&Path>| Art {
            path: backdrop.clone(),
            fallback: fallback.map(Path::to_path_buf),
        };

        let card = load(&art(Some(&poster_file))).unwrap();
        assert_eq!(card.dimensions(), (CARD_WIDTH, CARD_HEIGHT));
        assert!(missing(&load(&art(None)).map(DynamicImage::ImageRgb8)));
        assert!(missing(
            &load(&art(Some(&dir.0.join("none.jpg")))).map(DynamicImage::ImageRgb8)
        ));

        DynamicImage::ImageRgb8(RgbImage::from_pixel(780, 439, Rgb([10, 200, 10])))
            .save(&backdrop)
            .unwrap();
        let card = load(&art(Some(&poster_file))).unwrap();
        assert_eq!(card.dimensions(), (CARD_WIDTH, CARD_HEIGHT));
        assert!(
            card.get_pixel(0, 0).0[1] > 150,
            "a backdrop that exists wins over the poster"
        );
    }
}
