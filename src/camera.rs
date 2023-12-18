use std::time::Duration;

use axum::{
    body::{Body, Bytes},
    response::{IntoResponse, Response},
};
use drm_fourcc::DrmFourcc;
use eyre::{Result, WrapErr};
use libcamera::{
    camera::CameraConfigurationStatus,
    camera_manager::CameraManager,
    framebuffer::AsFrameBuffer,
    framebuffer_allocator::{FrameBuffer, FrameBufferAllocator},
    framebuffer_map::MemoryMappedFrameBuffer,
    logging::LoggingLevel,
    pixel_format::PixelFormat,
    stream::StreamRole,
};

const RGB888: PixelFormat = PixelFormat::new(DrmFourcc::Bgr888 as u32, 0);

fn get_image() -> Result<Bytes> {
    let mgr = CameraManager::new().unwrap();

    mgr.log_set_level("Camera", LoggingLevel::Error);

    let cameras = mgr.cameras();
    let cam = cameras.get(0).wrap_err("No camera found");

    println!("ID: {}", cam.id());

    println!("Properties: {:#?}", cam.properties());

    let mut config = cam
        .generate_configuration(&[StreamRole::StillCapture])
        .unwrap();

    config
        .get_mut(0)
        .wrap_err("No camera config generated")
        .set_pixel_format(RGB888);

    match config.validate() {
        CameraConfigurationStatus::Valid => println!("Camera configuration valid!"),
        CameraConfigurationStatus::Adjusted => {
            println!("Camera configuration was adjusted: {:#?}", config)
        }
        CameraConfigurationStatus::Invalid => {
            panic!("Error validating camera configuration")
        }
    };

    let mut cam = cam.acquire().wrap_err("Unable to acquire camera");
    cam.configure(&mut config)
        .wrap_err("Failed to configure active camera");
    let cfg = config.get(0).wrap_err("No config");

    let mut alloc = FrameBufferAllocator::new(&cam);
    let stream = cfg.stream().wrap_err("No camera stream");
    let buffers = alloc.alloc(&stream).unwrap();
    println!("Allocated {} buffers", buffers.len());

    // Convert FrameBuffer to MemoryMappedFrameBuffer, which allows reading &[u8]
    let buffers = buffers
        .into_iter()
        .take(1)
        .map(|buf| MemoryMappedFrameBuffer::new(buf).unwrap())
        .collect::<Vec<_>>();

    // Create capture requests and attach buffers
    let mut reqs = buffers
        .into_iter()
        .map(|buf| {
            let mut req = cam.create_request(None).unwrap();
            req.add_buffer(&stream, buf).unwrap();
            req
        })
        .collect::<Vec<_>>();

    // Completed capture requests are returned as a callback
    let (tx, rx) = std::sync::mpsc::channel();
    cam.on_request_completed(move |req| {
        tx.send(req).unwrap();
    });

    cam.start(None).unwrap();

    // Multiple requests can be queued at a time, but for this example we just want a single frame.
    cam.queue_request(reqs.pop().unwrap()).unwrap();

    println!("Waiting for camera request execution");
    let req = rx
        .recv_timeout(Duration::from_secs(2))
        .wrap_err("Camera request failed");

    println!("Camera request {:?} completed!", req);
    println!("Metadata: {:#?}", req.metadata());

    // Get framebuffer for our stream
    let framebuffer: &MemoryMappedFrameBuffer<FrameBuffer> = req.buffer(&stream).unwrap();
    println!("FrameBuffer metadata: {:#?}", framebuffer.metadata());

    let planes = framebuffer.data();
    let pixel_data = planes.get(0).unwrap();
    let pixel_len = framebuffer
        .metadata()
        .unwrap()
        .planes()
        .get(0)
        .unwrap()
        .bytes_used as usize;

    println!("Parsing image");

    let frame_size = cfg.get_size();
    let stride = cfg.get_stride() as usize;
    let pixel_data = {
        let row_width = (frame_size.width * 3) as usize;
        let mut pixel_data_parsed = vec![0; (frame_size.width * frame_size.height * 3) as usize];

        pixel_data[..pixel_len]
            .chunks_exact(stride)
            .enumerate()
            .for_each(|(i, chunk)| {
                pixel_data_parsed[row_width * i..row_width * (i + 1)]
                    .copy_from_slice(&chunk[..row_width]);
            });

        pixel_data_parsed
    };

    let image = image::RgbImage::from_raw(frame_size.width, frame_size.height, pixel_data)
        .ok_or(eyre::eyre!("Failed to parse image"))?;
    let output_format = image::ImageOutputFormat::Jpeg(90);

    let mut buffer = Vec::new();
    image.write_to(&mut buffer, output_format).unwrap();

    Ok(Bytes::from(image))
}

pub async fn current_view() -> impl IntoResponse {
    let bytes = spawn_blocking(get_image)
        .await
        .wrap_err("Failed to spawn blocking task");

    match bytes {
        Ok(bytes) => {
            let body = Body::from(bytes);

            Response::builder()
                .header("Content-Type", "image/jpeg")
                .body(body)
                .wrap_err("Failed to build response")
        }
        Err(e) => {
            let body = Body::from(format!("Error: {}", e));

            Response::builder()
                .status(500)
                .header("Content-Type", "text/plain")
                .body(body)
                .wrap_err("Failed to build response")
        }
    }
}
