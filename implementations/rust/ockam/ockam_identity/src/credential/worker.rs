use crate::authenticated_storage::AuthenticatedStorage;
use crate::credential::Credential;
use crate::{
    Identity, IdentityIdentifier, IdentitySecureChannelLocalInfo, IdentityVault, PublicIdentity,
};
use minicbor::Decoder;
use ockam_core::api::{Error, Id, Request, Response, ResponseBuilder, Status};
use ockam_core::async_trait;
use ockam_core::compat::{boxed::Box, string::ToString, vec::Vec};
use ockam_core::{Result, Routed, Worker};
use ockam_node::Context;
use tracing::{debug, error, trace, warn};

const TARGET: &str = "ockam::credential_exchange_worker::service";

/// Worker responsible for receiving and verifying other party's credentials
pub struct CredentialExchangeWorker<S: AuthenticatedStorage, V: IdentityVault> {
    authorities: Vec<PublicIdentity>,
    present_back: bool,
    identity: Identity<V, S>,
}

impl<S: AuthenticatedStorage, V: IdentityVault> CredentialExchangeWorker<S, V> {
    pub fn new(
        authorities: Vec<PublicIdentity>,
        present_back: bool,
        identity: Identity<V, S>,
    ) -> Self {
        Self {
            authorities,
            present_back,
            identity,
        }
    }
}

impl<S: AuthenticatedStorage, V: IdentityVault> CredentialExchangeWorker<S, V> {
    /// Create a generic bad request response.
    pub fn bad_request<'a>(id: Id, path: &'a str, msg: &'a str) -> ResponseBuilder<Error<'a>> {
        let e = Error::new(path).with_message(msg);
        Response::bad_request(id).body(e)
    }

    async fn handle_request(
        &mut self,
        _ctx: &mut Context,
        req: &Request<'_>,
        sender: IdentityIdentifier,
        dec: &mut Decoder<'_>,
    ) -> Result<Vec<u8>> {
        trace! {
            target: TARGET,
            id     = %req.id(),
            method = ?req.method(),
            path   = %req.path(),
            body   = %req.has_body(),
            "request"
        }

        use ockam_core::api::Method::*;
        let path = req.path();
        let path_segments = req.path_segments::<5>();
        let method = match req.method() {
            Some(m) => m,
            None => {
                return Ok(Response::bad_request(req.id())
                    .body("Invalid method")
                    .to_vec()?)
            }
        };

        let r = match (method, path_segments.as_slice()) {
            (Post, ["actions", "present"]) => {
                debug!(
                    "Received one-way credential presentation request from {}",
                    sender
                );
                let credential: Credential = dec.decode()?;

                let res = self
                    .identity
                    .receive_presented_credential(
                        sender.clone(),
                        credential,
                        self.authorities.iter(),
                    )
                    .await;

                match res {
                    Ok(()) => {
                        debug!("One-way credential presentation request processed successfully with {}", sender);
                        Response::ok(req.id()).to_vec()?
                    }
                    Err(err) => {
                        debug!(
                            "One-way credential presentation request processing error: {} for {}",
                            err, sender
                        );
                        Self::bad_request(req.id(), req.path(), &err.to_string()).to_vec()?
                    }
                }
            }
            (Post, ["actions", "present_mutual"]) => {
                debug!(
                    "Received mutual credential presentation request from {}",
                    sender
                );
                let credential: Credential = dec.decode()?;

                let res = self
                    .identity
                    .receive_presented_credential(
                        sender.clone(),
                        credential,
                        self.authorities.iter(),
                    )
                    .await;

                if let Err(err) = res {
                    debug!(
                        "Mutual credential presentation request processing error: {} from {}",
                        err, sender
                    );
                    Self::bad_request(req.id(), req.path(), &err.to_string()).to_vec()?
                } else {
                    debug!(
                        "Mutual credential presentation request processed successfully with {}",
                        sender
                    );
                    let credentials = self.identity.credential.read().await;
                    match credentials.as_ref() {
                        Some(p) if self.present_back => {
                            warn!("Mutual credential presentation request processed successfully with {}. Responding with own credential...", sender);
                            Response::ok(req.id()).body(p).to_vec()?
                        }
                        _ => {
                            warn!("Mutual credential presentation request processed successfully with {}. No credential to respond!", sender);
                            Response::ok(req.id()).to_vec()?
                        }
                    }
                }
            }

            // ==*== Catch-all for Unimplemented APIs ==*==
            _ => {
                warn!(%method, %path, "Called invalid endpoint");
                Response::bad_request(req.id())
                    .body(format!("Invalid endpoint: {}", path))
                    .to_vec()?
            }
        };
        Ok(r)
    }
}

#[async_trait]
impl<S: AuthenticatedStorage, V: IdentityVault> Worker for CredentialExchangeWorker<S, V> {
    type Message = Vec<u8>;
    type Context = Context;

    async fn handle_message(
        &mut self,
        ctx: &mut Self::Context,
        msg: Routed<Self::Message>,
    ) -> Result<()> {
        let mut dec = Decoder::new(msg.as_body());
        let req: Request = match dec.decode() {
            Ok(r) => r,
            Err(e) => {
                error!("failed to decode request: {:?}", e);
                return Ok(());
            }
        };

        let sender = IdentitySecureChannelLocalInfo::find_info(msg.local_message())?
            .their_identity_id()
            .clone();

        let r = match self.handle_request(ctx, &req, sender, &mut dec).await {
            Ok(r) => r,
            // If an error occurs, send a response with the error code so the listener can
            // fail fast instead of failing silently here and force the listener to timeout.
            Err(err) => {
                error!(?err, "Failed to handle message");
                Response::builder(req.id(), Status::InternalServerError)
                    .body(err.to_string())
                    .to_vec()?
            }
        };
        ctx.send(msg.return_route(), r).await
    }
}
