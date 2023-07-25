use chacha20::{
    cipher::{
        typenum::{UInt, UTerm, B0, B1},
        KeyIvInit, StreamCipher, StreamCipherCoreWrapper,
    },
    ChaCha20, ChaChaCore,
};
use futures::ready;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

type Cipher = StreamCipherCoreWrapper<ChaChaCore<UInt<UInt<UInt<UInt<UTerm, B1>, B0>, B1>, B0>>>;

// ----------------------------------------------------------------------------
// decoder
// ----------------------------------------------------------------------------
pub struct DecStream<R: AsyncRead + Unpin> {
    reader: R,
    // StreamCipherCoreWrapper<ChaCha20> not working..
    cipher: Cipher,
}

impl<R: AsyncRead + Unpin> DecStream<R> {
    pub fn new(reader: R, key: &[u8; 32], nonce: &[u8; 12]) -> DecStream<R> {
        DecStream {
            reader,
            cipher: ChaCha20::new(key.into(), nonce.into()),
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for DecStream<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        ready!(Pin::new(&mut self.reader).poll_read(cx, buf))?;
        tracing::trace!(
            "read<[raw] {}[bytes] {:?}",
            buf.filled().len(),
            &buf.filled()[0..std::cmp::min(buf.filled().len(), 128)]
        );
        if buf.filled().len() > 0 {
            self.cipher.apply_keystream(buf.filled_mut());
        }
        tracing::trace!(
            "read<[encrypted] {}[bytes] {:?}",
            buf.filled().len(),
            &buf.filled()[0..std::cmp::min(buf.filled().len(), 128)]
        );
        Poll::Ready(Ok(()))
    }
}

// ----------------------------------------------------------------------------
// encoder
// ----------------------------------------------------------------------------
pub struct EncStream<W: AsyncWrite + Unpin> {
    writer: W,
    cipher: Cipher,
    start: usize,
    end: usize,
    buffer: Option<Vec<u8>>,
}

impl<W: AsyncWrite + Unpin> EncStream<W> {
    pub fn new(writer: W, key: &[u8; 32], nonce: &[u8; 12]) -> EncStream<W> {
        EncStream {
            writer,
            cipher: ChaCha20::new(key.into(), nonce.into()),
            start: 0,
            end: 0,
            buffer: Some(vec![0u8; 4096]),
        }
    }
}

impl<W: AsyncWrite + Unpin> EncStream<W> {
    fn poll_write_inner(
        &mut self, // mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
        inner_buffer: &mut Vec<u8>,
    ) -> Poll<Result<usize, io::Error>> {
        let s = std::cmp::min(buf.len(), inner_buffer.len() - self.end);

        tracing::trace!("write<[raw] {:?}", &buf[..s]);
        inner_buffer[self.end..self.end + s].copy_from_slice(&buf[..s]);
        self.cipher
            .apply_keystream(&mut inner_buffer[self.end..self.end + s]);
        self.end += s;

        if self.start < self.end {
            tracing::trace!(
                "write<[encrypted] {:?}",
                &inner_buffer[self.start..self.end]
            );
            let n = ready!(
                Pin::new(&mut self.writer).poll_write(cx, &inner_buffer[self.start..self.end])
            )?;
            self.start += n;
        }
        if self.start >= self.end {
            self.start = 0;
            self.end = 0;
        }

        Poll::Ready(Ok(s))
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for EncStream<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let mut inner_buffer = self.buffer.take();
        let res = self.poll_write_inner(cx, buf, inner_buffer.as_mut().unwrap());
        self.buffer = inner_buffer;
        res
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        ready!(Pin::new(&mut self.writer).poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        ready!(Pin::new(&mut self.writer).poll_shutdown(cx))?;
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn enc() -> anyhow::Result<()> {
        let mut buffer = Vec::new();
        let mut writer = io::Cursor::new(&mut buffer);

        let key = [0x42; 32];
        let nonce = [0x24; 12];

        {
            let mut enc = EncStream::new(&mut writer, &key, &nonce);
            enc.write(b"hogehoge").await?;
        }

        println!("{:?}", buffer);

        let mut cipher = ChaCha20::new(&key.into(), &nonce.into());
        let mut b = buffer.clone();
        cipher.apply_keystream(&mut b);
        println!("{:?}", String::from_utf8(b));

        Ok(())
    }
}
