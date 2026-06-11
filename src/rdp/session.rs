/* rdp/session.rs
 *
 * Copyright 2026 Florian Richter
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Spawns the dedicated RDP worker thread (with its own tokio runtime) and
//! relays output events back to the GTK main loop.

use ironrdp::cliprdr::backend::CliprdrBackendFactory;
use ironrdp::session::GracefulDisconnectReason;
use ironrdp_client::config::Config;
use ironrdp_client::rdp::{DvcPipeProxyFactory, RdpClient, RdpInputEvent, RdpOutputEvent};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;

/// Spawns the RDP client on a dedicated thread with its own multi-threaded
/// tokio runtime. Output events are forwarded over `relay_tx` to the GTK main
/// loop; the relay is awaited after the client finishes so the terminal event
/// (e.g. `Terminated`) is delivered before the runtime drops.
///
/// `cancel_token` lets the caller abort the session while it is still
/// connecting. The connection handshake never reads the input channel, so a
/// graceful `RdpInputEvent::Close` only takes effect once the session is
/// active; cancelling the token drops the in-progress connection future
/// immediately and reports a graceful termination back to the UI.
pub fn spawn_rdp_session(
    config: Config,
    input_rx: UnboundedReceiver<RdpInputEvent>,
    cliprdr_factory: Option<Box<dyn CliprdrBackendFactory + Send>>,
    dvc_factory: DvcPipeProxyFactory,
    relay_tx: async_channel::Sender<RdpOutputEvent>,
    cancel_token: CancellationToken,
) {
    let (output_tx, mut output_rx) = tokio::sync::mpsc::channel::<RdpOutputEvent>(64);
    // Kept so the cancellation path can report a terminal event even though the
    // client owns the original sender.
    let cancel_output_tx = output_tx.clone();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();

        let client = RdpClient {
            config,
            output_event_sender: output_tx,
            input_event_receiver: input_rx,
            cliprdr_factory,
            dvc_pipe_proxy_factory: dvc_factory,
        };

        rt.block_on(async move {
            let relay_handle = tokio::spawn(async move {
                while let Some(event) = output_rx.recv().await {
                    if relay_tx.send(event).await.is_err() {
                        break;
                    }
                }
            });

            tokio::select! {
                () = client.run() => {}
                () = cancel_token.cancelled() => {
                    // Aborted before the session became active. The client
                    // future is dropped here (closing the socket); emit a
                    // terminal event so the UI returns to the disconnected state.
                    let _ = cancel_output_tx
                        .send(RdpOutputEvent::Terminated(Ok(GracefulDisconnectReason::UserInitiated)))
                        .await;
                }
            }
            // Wait for the relay to forward all remaining events (including Terminated)
            // before the runtime drops and cancels the task.
            drop(cancel_output_tx);
            let _ = relay_handle.await;
        });
    });
}
