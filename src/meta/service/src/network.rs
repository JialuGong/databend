// Copyright 2021 Datafuse Labs.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use common_base::containers::ItemManager;
use common_base::containers::Pool;
use common_meta_sled_store::openraft;
use common_meta_sled_store::openraft::MessageSummary;
use common_meta_types::protobuf::RaftRequest;
use common_meta_types::LogEntry;
use common_meta_types::NodeId;
use openraft::async_trait::async_trait;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::RaftNetwork;
use tonic::client::GrpcService;
use tonic::transport::channel::Channel;
use tracing::debug;
use tracing::info;

use crate::metrics::incr_meta_metrics_fail_connections_to_peer;
use crate::metrics::incr_meta_metrics_sent_bytes_to_peer;
use crate::metrics::incr_meta_metrics_sent_failure_to_peer;
use crate::metrics::incr_meta_metrics_snapshot_send_failures_to_peer;
use crate::metrics::incr_meta_metrics_snapshot_send_inflights_to_peer;
use crate::metrics::incr_meta_metrics_snapshot_send_success_to_peer;
use crate::metrics::sample_meta_metrics_snapshot_sent;
use crate::raft_client::RaftClient;
use crate::raft_client::RaftClientApi;
use crate::store::RaftStore;

struct ChannelManager {}

#[async_trait]
impl ItemManager for ChannelManager {
    type Key = String;
    type Item = Channel;
    type Error = tonic::transport::Error;

    async fn build(&self, addr: &Self::Key) -> Result<Channel, tonic::transport::Error> {
        tonic::transport::Endpoint::new(addr.clone())?
            .connect()
            .await
    }

    async fn check(&self, mut ch: Channel) -> Result<Channel, tonic::transport::Error> {
        futures::future::poll_fn(|cx| ch.poll_ready(cx)).await?;
        Ok(ch)
    }
}

pub struct Network {
    sto: Arc<RaftStore>,

    conn_pool: Pool<ChannelManager>,
}

impl Network {
    pub fn new(sto: Arc<RaftStore>) -> Network {
        let mgr = ChannelManager {};
        Network {
            sto,
            conn_pool: Pool::new(mgr, Duration::from_millis(50)),
        }
    }

    #[tracing::instrument(level = "debug", skip(self), fields(id=self.sto.id))]
    pub async fn make_client(&self, target: &NodeId) -> anyhow::Result<RaftClient> {
        let endpoint = self.sto.get_node_endpoint(target).await?;
        let addr = format!("http://{}", endpoint);

        debug!("connect: target={}: {}", target, addr);

        match self.conn_pool.get(&addr).await {
            Ok(channel) => {
                let client = RaftClientApi::new(*target, endpoint, channel);
                debug!("connected: target={}: {}", target, addr);

                Ok(client)
            }
            Err(err) => {
                incr_meta_metrics_fail_connections_to_peer(target, &endpoint.to_string());
                Err(err.into())
            }
        }
    }

    fn incr_meta_metrics_sent_bytes_to_peer(&self, target: &NodeId, message: &RaftRequest) {
        let bytes = message.data.len() as u64;
        incr_meta_metrics_sent_bytes_to_peer(target, bytes);
    }
}

#[async_trait]
impl RaftNetwork<LogEntry> for Network {
    #[tracing::instrument(level = "debug", skip_all, fields(id=self.sto.id, target=target))]
    async fn send_append_entries(
        &self,
        target: NodeId,
        rpc: AppendEntriesRequest<LogEntry>,
    ) -> anyhow::Result<AppendEntriesResponse> {
        debug!(
            "send_append_entries target: {}, rpc: {}",
            target,
            rpc.summary()
        );

        let mut client = self.make_client(&target).await?;

        let req = common_tracing::inject_span_to_tonic_request(rpc);

        self.incr_meta_metrics_sent_bytes_to_peer(&target, req.get_ref());

        let resp = client.append_entries(req).await;
        debug!("append_entries resp from: id={}: {:?}", target, resp);

        if resp.is_err() {
            incr_meta_metrics_sent_failure_to_peer(&target);
        }
        let resp = resp?;
        let mes = resp.into_inner();
        let resp = serde_json::from_str(&mes.data)?;

        Ok(resp)
    }

    #[tracing::instrument(level = "debug", skip_all, fields(id=self.sto.id, target=target))]
    async fn send_install_snapshot(
        &self,
        target: NodeId,
        rpc: InstallSnapshotRequest,
    ) -> anyhow::Result<InstallSnapshotResponse> {
        info!(
            "send_install_snapshot target: {}, rpc: {}",
            target,
            rpc.summary()
        );

        let start = Instant::now();
        let mut client = self.make_client(&target).await?;
        let req = common_tracing::inject_span_to_tonic_request(rpc);

        self.incr_meta_metrics_sent_bytes_to_peer(&target, req.get_ref());
        incr_meta_metrics_snapshot_send_inflights_to_peer(&target, 1);

        let resp = client.install_snapshot(req).await;
        info!("install_snapshot resp from: id={}: {:?}", target, resp);

        if resp.is_err() {
            incr_meta_metrics_sent_failure_to_peer(&target);
            incr_meta_metrics_snapshot_send_failures_to_peer(&target);
        } else {
            incr_meta_metrics_snapshot_send_success_to_peer(&target);
        }
        incr_meta_metrics_snapshot_send_inflights_to_peer(&target, -1);

        let resp = resp?;
        let mes = resp.into_inner();
        let resp = serde_json::from_str(&mes.data)?;

        sample_meta_metrics_snapshot_sent(&target, start.elapsed().as_secs() as f64);

        Ok(resp)
    }

    #[tracing::instrument(level = "debug", skip_all, fields(id=self.sto.id, target=target))]
    async fn send_vote(&self, target: NodeId, rpc: VoteRequest) -> anyhow::Result<VoteResponse> {
        info!("send_vote: target: {} rpc: {}", target, rpc.summary());

        let mut client = self.make_client(&target).await?;
        let req = common_tracing::inject_span_to_tonic_request(rpc);

        self.incr_meta_metrics_sent_bytes_to_peer(&target, req.get_ref());

        let resp = client.vote(req).await;
        info!("vote: resp from target={} {:?}", target, resp);

        if resp.is_err() {
            incr_meta_metrics_sent_failure_to_peer(&target);
        }

        let resp = resp?;
        let mes = resp.into_inner();
        let resp = serde_json::from_str(&mes.data)?;

        Ok(resp)
    }
}
