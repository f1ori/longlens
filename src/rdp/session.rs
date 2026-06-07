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
use ironrdp_client::config::Config;
use ironrdp_client::rdp::{DvcPipeProxyFactory, RdpClient, RdpInputEvent, RdpOutputEvent};
use tokio::sync::mpsc::UnboundedReceiver;

/// Spawns the RDP client on a dedicated thread with its own multi-threaded
/// tokio runtime. Output events are forwarded over `relay_tx` to the GTK main
/// loop; the relay is awaited after the client finishes so the terminal event
/// (e.g. `Terminated`) is delivered before the runtime drops.
pub fn spawn_rdp_session(
    config: Config,
    input_rx: UnboundedReceiver<RdpInputEvent>,
    cliprdr_factory: Option<Box<dyn CliprdrBackendFactory + Send>>,
    dvc_factory: DvcPipeProxyFactory,
    relay_tx: async_channel::Sender<RdpOutputEvent>,
) {
    let (output_tx, mut output_rx) = tokio::sync::mpsc::channel::<RdpOutputEvent>(64);

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

            client.run().await;
            // Wait for the relay to forward all remaining events (including Terminated)
            // before the runtime drops and cancels the task.
            let _ = relay_handle.await;
        });
    });
}
