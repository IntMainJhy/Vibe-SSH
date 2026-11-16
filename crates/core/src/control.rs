use russh::{ChannelWriteHalf, client};
use std::time::Duration;
use tokio::sync::Mutex;
use vibe_ssh_protocol::{InputLease, InputOwner};

use crate::{Error, Result};

struct State {
    lease: InputLease,
    channel: ChannelWriteHalf<client::Msg>,
}

/// The lease check and transport write share one serialization boundary.
pub struct ControlledInput {
    state: Mutex<State>,
}

impl ControlledInput {
    pub(crate) fn new(channel: ChannelWriteHalf<client::Msg>) -> (Self, InputLease) {
        let lease = InputLease::issue(0, InputOwner::Agent);
        (
            Self {
                state: Mutex::new(State {
                    lease: lease.clone(),
                    channel,
                }),
            },
            lease,
        )
    }

    /// Trusted in-process controller operation; IPC authentication comes in M3.
    pub async fn transfer(&self, owner: InputOwner) -> Result<InputLease> {
        let mut state = self.state.lock().await;
        let epoch = state
            .lease
            .epoch()
            .checked_add(1)
            .ok_or(Error::EpochExhausted)?;
        state.lease = InputLease::issue(epoch, owner);
        Ok(state.lease.clone())
    }

    pub async fn write(&self, lease: &InputLease, data: &[u8]) -> Result<()> {
        if data.len() > 32_768 {
            return Err(Error::InvalidRequest(
                "input chunks must not exceed 32768 bytes",
            ));
        }
        let state = self.state.lock().await;
        if &state.lease != lease {
            return Err(Error::ControlRevoked);
        }
        // Keep the lock through submission: takeover cannot acknowledge in between.
        tokio::time::timeout(Duration::from_secs(3), state.channel.data(data))
            .await
            .map_err(|_| Error::InputDeliveryUnknown)??;
        Ok(())
    }

    pub async fn resize(&self, lease: &InputLease, columns: u32, rows: u32) -> Result<()> {
        if columns == 0 || rows == 0 || columns > 1000 || rows > 1000 {
            return Err(Error::InvalidRequest(
                "terminal dimensions must be 1..=1000",
            ));
        }
        let state = self.state.lock().await;
        if &state.lease != lease {
            return Err(Error::ControlRevoked);
        }
        state.channel.window_change(columns, rows, 0, 0).await?;
        Ok(())
    }

    pub(crate) async fn close(&self) -> Result<()> {
        self.state.lock().await.channel.close().await?;
        Ok(())
    }
}
