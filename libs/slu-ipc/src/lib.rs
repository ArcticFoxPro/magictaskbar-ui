pub mod error;
pub mod messages;

use std::{
    future::Future,
    io::{BufRead, Write},
    sync::Arc,
};

use interprocess::os::windows::{
    named_pipe::{
        pipe_mode::Bytes, tokio::DuplexPipeStream as AsyncDuplexPipeStream, DuplexPipeStream,
        PipeListenerOptions,
    },
    security_descriptor::{AsSecurityDescriptorMutExt, SecurityDescriptor},
};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    error::Result,
    messages::{AppMessage, IpcResponse, SvcAction, SvcMessage},
};

/// https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-control
static SE_DACL_PROTECTED: u16 = 4096u16;

const END_OF_TRANSMISSION_BLOCK: u8 = 0x17;

pub trait IPC {
    const PATH: &'static str;

    #[allow(async_fn_in_trait)]
    async fn server_process_id() -> Result<u32> {
        let stream = AsyncDuplexPipeStream::connect_by_path(Self::PATH).await?;
        let pid = stream.server_process_id()?;
        write_to_ipc_stream(&stream, &[]).await?;
        Ok(pid)
    }

    /// returns the server process id
    fn test_connection() -> Result<()> {
        let stream = DuplexPipeStream::connect_by_path(Self::PATH)?;
        let response = send_to_ipc_stream(&stream, &[])?;
        response.ok()
    }

    fn can_stablish_connection() -> bool {
        Self::test_connection().is_ok()
    }
}

pub struct ServiceIpc {
    _priv: (),
}

impl IPC for ServiceIpc {
    const PATH: &'static str = r"\\.\pipe\magictaskbar-service";
}

impl ServiceIpc {
    pub fn start<R, F>(cb: F) -> Result<()>
    where
        R: Future<Output = IpcResponse> + Send + Sync,
        F: Fn(SvcAction) -> R + Send + Sync + 'static,
    {
        let mut sd = SecurityDescriptor::new()?;
        unsafe { sd.set_dacl(std::ptr::null_mut(), false)? };
        let _ = sd.set_control(SE_DACL_PROTECTED, SE_DACL_PROTECTED);

        let listener = PipeListenerOptions::new()
            .path(Self::PATH)
            .security_descriptor(Some(sd))
            .create_tokio_duplex::<Bytes>()?;

        tokio::spawn(async move {
            let callback = Arc::new(cb);
            while let Ok(stream) = listener.accept().await {
                let callback = callback.clone();
                tokio::spawn(async move {
                    if let Err(err) = Self::process_connection(stream, callback).await {
                        let is_pipe_closing = err.to_string().contains("232");
                        log::error!("IPC connection error: {err}");

                        if !is_pipe_closing {
                            log::error!("IPC connection error: {err}");
                        }
                    }
                });
            }
        });
        Ok(())
    }

    async fn process_connection<F, R>(
        stream: AsyncDuplexPipeStream<Bytes>,
        cb: Arc<F>,
    ) -> Result<()>
    where
        R: Future<Output = IpcResponse> + Send + Sync,
        F: Fn(SvcAction) -> R + Send + Sync + 'static,
    {
        let data = read_from_ipc_stream(&stream).await?;

        if data.is_empty() {
            return Self::response_to_client(&stream, IpcResponse::Success).await;
        }

        let message = SvcMessage::from_bytes(&data)?;
        if !message.is_signature_valid() {
            Self::response_to_client(
                &stream,
                IpcResponse::Err("Unauthorized connection".to_owned()),
            )
            .await?;
            return Ok(());
        }

        Self::response_to_client(&stream, cb(message.action).await).await?;
        Ok(())
    }

    async fn response_to_client(
        stream: &AsyncDuplexPipeStream<Bytes>,
        res: IpcResponse,
    ) -> Result<()> {
        write_to_ipc_stream(stream, &res.to_bytes()?).await
    }

    pub async fn send(message: SvcAction) -> Result<IpcResponse> {
        let stream = AsyncDuplexPipeStream::connect_by_path(Self::PATH).await?;
        let data = SvcMessage {
            token: SvcMessage::signature().to_string(),
            action: message,
        }
        .to_bytes()?;
        async_send_to_ipc_stream(&stream, &data).await
    }

    /// Synchronous version of send for use in non-async contexts (e.g., WebView2 event handlers)
    pub fn send_sync(message: SvcAction) -> Result<IpcResponse> {
        let stream = DuplexPipeStream::connect_by_path(Self::PATH)?;
        let data = SvcMessage {
            token: SvcMessage::signature().to_string(),
            action: message,
        }
        .to_bytes()?;
        send_to_ipc_stream(&stream, &data)
    }
}

pub struct AppIpc {
    _priv: (),
}

impl IPC for AppIpc {
    const PATH: &'static str = r"\\.\pipe\magictaskbar-ui";
}

impl AppIpc {
    pub fn start<F>(cb: F) -> Result<()>
    where
        F: Fn(AppMessage) -> IpcResponse + Send + Sync + 'static,
    {
        let mut sd = SecurityDescriptor::new()?;
        unsafe { sd.set_dacl(std::ptr::null_mut(), false)? };
        let _ = sd.set_control(SE_DACL_PROTECTED, SE_DACL_PROTECTED);

        let listener = PipeListenerOptions::new()
            .path(Self::PATH)
            .security_descriptor(Some(sd))
            .create_tokio_duplex::<Bytes>()?;

        tokio::spawn(async move {
            let callback = Arc::new(cb);
            while let Ok(stream) = listener.accept().await {
                let callback = callback.clone();
                tokio::spawn(async move {
                    if let Err(err) = Self::process_connection(stream, callback).await {
                        log::error!("IPC connection error: {err}");
                    }
                });
            }
        });
        Ok(())
    }

    async fn process_connection<F>(stream: AsyncDuplexPipeStream<Bytes>, cb: Arc<F>) -> Result<()>
    where
        F: Fn(AppMessage) -> IpcResponse,
    {
        let data = read_from_ipc_stream(&stream).await?;

        if data.is_empty() {
            return Self::response_to_client(&stream, IpcResponse::Success).await;
        }

        let message = AppMessage::from_bytes(&data)?;

        // Handle different AppMessage variants
        match message {
            _ => {
                // For all messages, use the callback
                Self::response_to_client(&stream, cb(message)).await?;
            }
        }

        Ok(())
    }

    async fn response_to_client(
        stream: &AsyncDuplexPipeStream<Bytes>,
        res: IpcResponse,
    ) -> Result<()> {
        write_to_ipc_stream(stream, &res.to_bytes()?).await
    }

    pub async fn send(message: AppMessage) -> Result<()> {
        let stream = AsyncDuplexPipeStream::connect_by_path(Self::PATH).await?;
        async_send_to_ipc_stream(&stream, &message.to_bytes()?)
            .await?
            .ok()
    }

    pub fn send_sync(message: &AppMessage) -> Result<()> {
        let stream = DuplexPipeStream::connect_by_path(Self::PATH)?;
        let data = message.to_bytes()?;
        send_to_ipc_stream_blocking(&stream, &data)?;
        Ok(())
    }
}

async fn read_from_ipc_stream(stream: &AsyncDuplexPipeStream<Bytes>) -> Result<Vec<u8>> {
    let mut stream = stream;

    let mut buf = Vec::new();
    let mut byte = [0u8; 1];

    loop {
        stream.read_exact(&mut byte).await?;

        if byte[0] == END_OF_TRANSMISSION_BLOCK {
            break;
        }

        buf.push(byte[0]);
    }

    Ok(buf)
}

async fn write_to_ipc_stream(stream: &AsyncDuplexPipeStream<Bytes>, buf: &[u8]) -> Result<()> {
    let mut stream = stream;

    stream.write_all(buf).await?;
    stream.write_all(&[END_OF_TRANSMISSION_BLOCK]).await?;
    stream.flush().await?;

    Ok(())
}

async fn async_send_to_ipc_stream(
    stream: &AsyncDuplexPipeStream<Bytes>,
    buf: &[u8],
) -> Result<IpcResponse> {
    write_to_ipc_stream(stream, buf).await?;
    let buf = read_from_ipc_stream(stream).await?;
    IpcResponse::from_bytes(&buf)
}

/// blocking version to test connections without needed of tokio runtime
fn send_to_ipc_stream(stream: &DuplexPipeStream<Bytes>, buf: &[u8]) -> Result<IpcResponse> {
    send_to_ipc_stream_blocking(stream, buf)
}

fn send_to_ipc_stream_blocking(
    stream: &DuplexPipeStream<Bytes>,
    buf: &[u8],
) -> Result<IpcResponse> {
    let mut writter = std::io::BufWriter::new(stream);
    writter.write_all(buf)?;
    writter.write_all(&[END_OF_TRANSMISSION_BLOCK])?;
    writter.flush()?;

    let mut reader = std::io::BufReader::new(stream);
    let mut buf = Vec::new();
    reader.read_until(END_OF_TRANSMISSION_BLOCK, &mut buf)?;
    buf.pop();

    IpcResponse::from_bytes(&buf)
}
