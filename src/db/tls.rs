//! TLS for Postgres over tokio-rustls on ring. Prefer and Require share one config that never
//! verifies the certificate, as libpq does for those modes (verify-ca/verify-full are out of scope).

use std::io;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, InvalidDnsNameError, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::tls::{ChannelBinding, MakeTlsConnect, TlsConnect};

/// The `MakeTlsConnect` every Postgres connect uses; Disable never calls it.
#[derive(Clone)]
pub struct Tls(Arc<ClientConfig>);

impl Tls {
    pub fn get() -> Self {
        static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
        Self(
            CONFIG
                .get_or_init(|| {
                    let provider = Arc::new(rustls::crypto::ring::default_provider());
                    let config = ClientConfig::builder_with_provider(provider.clone())
                        .with_safe_default_protocol_versions()
                        .expect("ring supports the default TLS versions")
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
                        .with_no_client_auth();
                    Arc::new(config)
                })
                .clone(),
        )
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send + 'static> MakeTlsConnect<S> for Tls {
    type Stream = TlsStream<S>;
    type TlsConnect = Connect;
    type Error = InvalidDnsNameError;

    fn make_tls_connect(&mut self, domain: &str) -> Result<Connect, InvalidDnsNameError> {
        Ok(Connect(
            self.0.clone(),
            ServerName::try_from(domain.to_string())?,
        ))
    }
}

pub struct Connect(Arc<ClientConfig>, ServerName<'static>);

impl<S: AsyncRead + AsyncWrite + Unpin + Send + 'static> TlsConnect<S> for Connect {
    type Stream = TlsStream<S>;
    type Error = io::Error;
    type Future = BoxFuture<'static, io::Result<TlsStream<S>>>;

    fn connect(self, stream: S) -> Self::Future {
        Box::pin(async move {
            tokio_rustls::TlsConnector::from(self.0)
                .connect(self.1, stream)
                .await
                .map(TlsStream)
        })
    }
}

pub struct TlsStream<S>(tokio_rustls::client::TlsStream<S>);

impl<S: AsyncRead + AsyncWrite + Unpin> tokio_postgres::tls::TlsStream for TlsStream<S> {
    fn channel_binding(&self) -> ChannelBinding {
        ChannelBinding::none()
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for TlsStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for TlsStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

/// Accepts any certificate, but still checks the handshake signatures against it.
#[derive(Debug)]
struct AnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tls_is_ring_only() {
        let lock = include_str!("../../Cargo.lock");
        assert!(
            !lock.contains("aws-lc"),
            "Cargo.lock pulls in aws-lc; keep rustls and tokio-rustls on ring"
        );
    }
}
