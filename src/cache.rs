use std::time::{Duration, Instant};

use image::RgbImage;

#[derive(Debug, Clone)]
pub struct ImageCache {
    image: RgbImage,
    created_at: Instant,
}

const CACHE_AGE: Duration = Duration::from_secs(10);

impl ImageCache {
    pub fn new(image: RgbImage) -> Self {
        Self {
            image,
            created_at: Instant::now(),
        }
    }

    pub fn stale(&self) -> bool {
        self.created_at.elapsed() > CACHE_AGE
    }

    pub fn image(&self) -> RgbImage {
        self.image.clone()
    }
}
