use std::{io::Cursor, time::Duration};

use axum::{
    body::{Body, Bytes},
    extract::{Path, Request},
    response::{IntoResponse, Response},
};
use drm_fourcc::DrmFourcc;
use eyre::{ContextCompat, OptionExt, Report, Result, WrapErr};
use image::{ImageFormat, RgbImage};
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
use tokio::task::spawn_blocking;

const RGB888: PixelFormat = PixelFormat::new(DrmFourcc::Bgr888 as u32, 0);

fn get_image() -> Result<RgbImage> {
    let mgr = CameraManager::new().unwrap();

    mgr.log_set_level("Camera", LoggingLevel::Error);

    let cameras = mgr.cameras();
    let cam = cameras.get(0).ok_or_eyre("No camera found")?;

    println!("ID: {}", cam.id());

    println!("Properties: {:#?}", cam.properties());

    let mut config = cam
        .generate_configuration(&[StreamRole::StillCapture])
        .unwrap();

    config
        .get_mut(0)
        .ok_or_eyre("No camera config generated")?
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

    let mut cam = cam.acquire().wrap_err("Unable to acquire camera")?;
    cam.configure(&mut config)
        .wrap_err("Failed to configure active camera")?;
    let cfg = config.get(0).ok_or_eyre("No config")?;

    let mut alloc = FrameBufferAllocator::new(&cam);
    let stream = cfg.stream().ok_or_eyre("No camera stream")?;
    let buffers = alloc.alloc(&stream).wrap_err("Failed to allocate buffer")?;
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
            let mut req = cam
                .create_request(None)
                .wrap_err("Failed to create camera request")?;
            req.add_buffer(&stream, buf)
                .wrap_err("Can't add buffer to request")?;

            Ok::<_, Report>(req)
        })
        .collect::<Result<Vec<_>, _>>()?;

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
        .wrap_err("Camera request failed")?;

    println!("Camera request {:?} completed!", req);
    println!("Metadata: {:#?}", req.metadata());

    // Get framebuffer for our stream
    let framebuffer: &MemoryMappedFrameBuffer<FrameBuffer> =
        req.buffer(&stream).ok_or_eyre("No buffer found")?;
    println!("FrameBuffer metadata: {:#?}", framebuffer.metadata());

    let planes = framebuffer.data();
    let pixel_data = planes.get(0).ok_or_eyre("No planes in camera response")?;
    let pixel_len = framebuffer
        .metadata()
        .ok_or_eyre("Got response withoud metadata")?
        .planes()
        .get(0)
        .ok_or_eyre("No planes in camera response")?
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

    Ok(
        RgbImage::from_raw(frame_size.width, frame_size.height, pixel_data)
            .ok_or_eyre("Failed to parse image")?,
    )
}

pub async fn current_view(Path(extension): Path<String>) -> impl IntoResponse {
    let image_res = match spawn_blocking(get_image)
        .await
        .wrap_err("Failed to spawn blocking task")
    {
        Ok(bytes_res) => bytes_res,
        Err(e) => {
            let body = Body::from(format!("Error: {}", e));

            return Response::builder()
                .status(500)
                .header("Content-Type", "text/plain")
                .body(body)
                .expect("Failed to build response");
        }
    };

    let image = match image_res {
        Ok(image) => image,
        Err(e) => {
            let body = Body::from(format!("Error: {}", e));

            return Response::builder()
                .status(500)
                .header("Content-Type", "text/plain")
                .body(body)
                .expect("Failed to build response");
        }
    };

    let format = match extension.as_str() {
        "jpg" => ImageFormat::Jpeg,
        "png" => ImageFormat::Png,
        "gif" => ImageFormat::Gif,
        "bmp" => ImageFormat::Bmp,
        "ico" => ImageFormat::Ico,
        "tiff" => ImageFormat::Tiff,
        "webp" => ImageFormat::WebP,
        "qoi" => ImageFormat::Qoi,
        _ => ImageFormat::Jpeg,
    };

    let content_type = match format {
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Png => "image/png",
        ImageFormat::Gif => "image/gif",
        ImageFormat::Bmp => "image/bmp",
        ImageFormat::Ico => "image/ico",
        ImageFormat::Tiff => "image/tiff",
        ImageFormat::WebP => "image/webp",
        ImageFormat::Qoi => "image/qoi",
        _ => "image/jpeg",
    };

    let mut body = Cursor::new(Vec::new());
    image
        .write_to(&mut body, format)
        .expect("Failed to write image to buffer");

    let body = Body::from(body.into_inner());

    Response::builder()
        .header("Content-Type", content_type)
        .body(body)
        .expect("Failed to build response")
}
