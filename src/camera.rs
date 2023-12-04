use std::time::Duration;

use axum::{
    body::{Body, Bytes},
    response::{IntoResponse, Response},
};
use libcamera::{
    camera::CameraConfigurationStatus,
    camera_manager::CameraManager,
    framebuffer::AsFrameBuffer,
    framebuffer_allocator::{FrameBuffer, FrameBufferAllocator},
    framebuffer_map::MemoryMappedFrameBuffer,
    pixel_format::PixelFormat,
    stream::StreamRole,
};
use tokio::task::spawn_blocking;

const PIXEL_FORMAT_MJPEG: PixelFormat =
    PixelFormat::new(u32::from_le_bytes([b'M', b'J', b'P', b'G']), 0);

fn get_image() -> Bytes {
    let camera_manager = CameraManager::new().expect("Failed to create camera manager");
    let cameras = camera_manager.cameras();

    let camera = cameras.get(0).expect("No cameras available");

    let mut camera_device = camera.acquire().expect("Failed to acquire camera");
    let mut config = camera_device
        .generate_configuration(&[StreamRole::StillCapture])
        .expect("Failed to generate configuration");

    config
        .get_mut(0)
        .expect("Failed to get configuration")
        .set_pixel_format(PIXEL_FORMAT_MJPEG);

    match config.validate() {
        CameraConfigurationStatus::Valid => println!("Camera configuration valid!"),
        CameraConfigurationStatus::Adjusted => {
            println!("Camera configuration was adjusted: {:#?}", config)
        }
        CameraConfigurationStatus::Invalid => panic!("Error validating camera configuration"),
    }

    camera_device
        .configure(&mut config)
        .expect("Unable to configure camera");

    let mut alloc = FrameBufferAllocator::new(&camera_device);

    // Allocate frame buffers for the stream
    let stream_config = config.get(0).expect("Failed to get configuration");
    let stream = stream_config.stream().expect("Failed to get stream");
    let buffers = alloc.alloc(&stream).expect("Failed to allocate buffers");

    println!("Allocated {} buffers", buffers.len());

    // Convert FrameBuffer to MemoryMappedFrameBuffer, which allows reading &[u8]
    let buffers = buffers
        .into_iter()
        .map(|buf| MemoryMappedFrameBuffer::new(buf).unwrap())
        .collect::<Vec<_>>();

    // Create capture requests and attach buffers
    let mut reqs = buffers
        .into_iter()
        .map(|buf| {
            let mut req = camera_device.create_request(None).unwrap();
            req.add_buffer(&stream, buf).unwrap();
            req
        })
        .collect::<Vec<_>>();

    // Completed capture requests are returned as a callback
    let (tx, rx) = std::sync::mpsc::channel();
    camera_device.on_request_completed(move |req| {
        tx.send(req).unwrap();
    });

    camera_device.start(None).unwrap();

    // Multiple requests can be queued at a time, but for this example we just want a single frame.
    camera_device.queue_request(reqs.pop().unwrap()).unwrap();

    println!("Waiting for camera request execution");
    let req = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Camera request failed");

    println!("Camera request {:?} completed!", req);
    println!("Metadata: {:#?}", req.metadata());

    // Get framebuffer for our stream
    let framebuffer: &MemoryMappedFrameBuffer<FrameBuffer> = req.buffer(&stream).unwrap();
    println!("FrameBuffer metadata: {:#?}", framebuffer.metadata());

    // MJPEG format has only one data plane containing encoded jpeg data with all the headers
    let planes = framebuffer.data();
    let jpeg_data = planes.get(0).unwrap();
    // Actual JPEG-encoded data will be smalled than framebuffer size, its length can be obtained from metadata.
    let jpeg_len = framebuffer
        .metadata()
        .unwrap()
        .planes()
        .get(0)
        .unwrap()
        .bytes_used as usize;

    Bytes::copy_from_slice(&jpeg_data[..jpeg_len])
}

pub async fn current_view() -> impl IntoResponse {
    let bytes = spawn_blocking(get_image)
        .await
        .expect("Failed to spawn blocking task");
    let body = Body::from(bytes);

    Response::builder()
        .header("Content-Type", "image/jpeg")
        .body(body)
        .expect("Failed to build response")
}
