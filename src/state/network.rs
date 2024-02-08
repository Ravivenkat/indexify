use std::{error::Error, fmt::Display, sync::Arc};

use anyerror::AnyError;
use openraft::{
    error::{NetworkError, RemoteError, Unreachable},
    network::{RaftNetwork, RaftNetworkFactory},
    raft::{
        AppendEntriesRequest,
        AppendEntriesResponse,
        InstallSnapshotRequest,
        InstallSnapshotResponse,
        VoteRequest,
        VoteResponse,
    },
    BasicNode,
};

use super::{raft_client::RaftClient, NodeId, TypeConfig};
use crate::{
    grpc_helper::GrpcHelper,
    state::typ::{InstallSnapshotError, RPCError, RaftError},
};

pub struct Network {
    raft_client: Arc<RaftClient>,
}

impl Default for Network {
    fn default() -> Self {
        Self::new()
    }
}

impl Network {
    pub fn new() -> Self {
        Self {
            raft_client: Arc::new(RaftClient::new()),
        }
    }
}

impl RaftNetworkFactory<TypeConfig> for Network {
    type Network = NetworkConnection;

    async fn new_client(&mut self, target: NodeId, node: &BasicNode) -> Self::Network {
        NetworkConnection {
            target,
            target_node: node.clone(),
            raft_client: self.raft_client.clone(),
        }
    }
}

pub struct NetworkConnection {
    target: NodeId,
    target_node: BasicNode,
    raft_client: Arc<RaftClient>,
}

impl NetworkConnection {
    fn status_to_unreachable<E>(&self, status: tonic::Status) -> RPCError<RaftError<E>>
    where
        E: Error,
    {
        RPCError::Unreachable(Unreachable::new(&status))
    }

    /// Wrap a RaftError with RPCError
    pub(crate) fn to_rpc_err<E: Error>(&self, e: RaftError<E>) -> RPCError<RaftError<E>> {
        let remote_err = RemoteError::new_with_node(self.target, self.target_node.clone(), e);
        RPCError::RemoteError(remote_err)
    }
}

impl RaftNetwork<TypeConfig> for NetworkConnection {
    async fn send_append_entries(
        &mut self,
        req: AppendEntriesRequest<TypeConfig>,
    ) -> Result<AppendEntriesResponse<NodeId>, RPCError<RaftError>> {
        let mut client = self
            .raft_client
            .clone()
            .get(&self.target_node.addr)
            .await
            .map_err(|e| self.status_to_unreachable(tonic::Status::aborted(e.to_string())))?;

        let raft_req = GrpcHelper::encode_raft_request(&req).map_err(|e| Unreachable::new(&e))?;
        let req = GrpcHelper::into_req(raft_req);

        let grpc_res = client.append_entries(req).await;

        let resp = grpc_res.map_err(|e| self.status_to_unreachable(e))?;

        let raft_res = GrpcHelper::parse_raft_reply(resp)
            .map_err(|serde_err| new_net_err(&serde_err, || "parse append_entries reply"))?;

        raft_res.map_err(|e| self.to_rpc_err(e))
    }

    async fn send_install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<TypeConfig>,
    ) -> Result<InstallSnapshotResponse<NodeId>, RPCError<RaftError<InstallSnapshotError>>> {
        let mut client = self
            .raft_client
            .get(&self.target_node.addr)
            .await
            .map_err(|e| self.status_to_unreachable(tonic::Status::aborted(e.to_string())))?;

        let raft_req = GrpcHelper::encode_raft_request(&req).map_err(|e| Unreachable::new(&e))?;
        let req = GrpcHelper::into_req(raft_req);

        let grpc_res = client.install_snapshot(req).await;

        let resp = grpc_res.map_err(|e| self.status_to_unreachable(e))?;

        let raft_res = GrpcHelper::parse_raft_reply(resp)
            .map_err(|serde_err| new_net_err(&serde_err, || "parse install_snapshot reply"))?;

        raft_res.map_err(|e| self.to_rpc_err(e))
    }

    async fn send_vote(
        &mut self,
        req: VoteRequest<NodeId>,
    ) -> Result<VoteResponse<NodeId>, RPCError<RaftError>> {
        let mut client = self
            .raft_client
            .get(&self.target_node.addr)
            .await
            .map_err(|e| self.status_to_unreachable(tonic::Status::aborted(e.to_string())))?;

        let raft_req = GrpcHelper::encode_raft_request(&req).map_err(|e| Unreachable::new(&e))?;

        let req = GrpcHelper::into_req(raft_req);

        let grpc_res = client.vote(req).await;

        let resp = grpc_res.map_err(|e| self.status_to_unreachable(e))?;

        let raft_res = GrpcHelper::parse_raft_reply(resp)
            .map_err(|serde_err| new_net_err(&serde_err, || "parse vote reply"))?;

        raft_res.map_err(|e| self.to_rpc_err(e))
    }
}

fn new_net_err<D: Display>(e: &(impl Error + 'static), msg: impl FnOnce() -> D) -> NetworkError {
    NetworkError::new(&AnyError::new(e).add_context(msg))
}
