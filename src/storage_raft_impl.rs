//! OpenRaft storage trait implementation for SurrealStorage.
//!
//! This module is intentionally isolated from `storage.rs` so core storage logic
//! remains readable while trait glue evolves.

use crate::state::{AppliedState, VotingState};
use crate::storage::SurrealStorage;
use crate::types::{KVResponse, RaftTypeConfig};
use openraft::storage::{
    IOFlushed, LogState, RaftLogReader, RaftLogStorage, RaftSnapshotBuilder, RaftStateMachine,
    Snapshot, SnapshotMeta,
};
use openraft::{EntryPayload, OptionalSend, StoredMembership, Vote};
use std::fmt::Debug;
use std::io;
use std::ops::{Bound, RangeBounds};
use tokio_stream::StreamExt;

fn range_start<RB: RangeBounds<u64>>(range: &RB) -> u64 {
    match range.start_bound() {
        Bound::Included(v) => *v,
        Bound::Excluded(v) => v.saturating_add(1),
        Bound::Unbounded => 0,
    }
}

fn range_end_inclusive<RB: RangeBounds<u64>>(range: &RB) -> u64 {
    match range.end_bound() {
        Bound::Included(v) => *v,
        Bound::Excluded(v) => v.saturating_sub(1),
        Bound::Unbounded => u64::MAX,
    }
}

impl RaftLogReader<RaftTypeConfig> for SurrealStorage {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<<RaftTypeConfig as openraft::RaftTypeConfig>::Entry>, io::Error> {
        let start = range_start(&range);
        let end = range_end_inclusive(&range);
        if end < start {
            return Ok(Vec::new());
        }

        let logs = self.raft_entries.read().await;
        Ok(logs
            .range(start..=end)
            .map(|(_, ent)| ent.clone())
            .collect::<Vec<_>>())
    }

    async fn read_vote(
        &mut self,
    ) -> Result<Option<openraft::type_config::alias::VoteOf<RaftTypeConfig>>, io::Error> {
        let txn = self.tree().begin().map_err(io::Error::other)?;
        if let Some(bytes) = txn.get(b"raft_vote_v2").map_err(io::Error::other)? {
            let vote: Vote<RaftTypeConfig> =
                postcard::from_bytes(&bytes).map_err(io::Error::other)?;
            return Ok(Some(vote));
        }

        let v = self.metadata().get_voting_state().await;
        Ok(v.voted_for.map(|id| Vote::new(v.current_term, id)))
    }
}

impl RaftSnapshotBuilder<RaftTypeConfig> for SurrealStorage {
    async fn build_snapshot(&mut self) -> Result<Snapshot<RaftTypeConfig>, io::Error> {
        let applied = self.metadata().get_applied_state().await;
        let mut snap = self
            .build_snapshot_auto(applied.last_applied_term)
            .await
            .map_err(io::Error::other)?;

        let membership = self.last_membership.read().await.clone();
        snap.meta.last_membership = membership;
        *self.current_snapshot.write().await = Some(snap.clone());
        Ok(snap)
    }
}

impl RaftStateMachine<RaftTypeConfig> for SurrealStorage {
    type SnapshotBuilder = SurrealStorage;

    async fn applied_state(
        &mut self,
    ) -> Result<
        (
            Option<openraft::type_config::alias::LogIdOf<RaftTypeConfig>>,
            StoredMembership<RaftTypeConfig>,
        ),
        io::Error,
    > {
        let st = self.metadata().get_applied_state().await;
        let last = if st.last_applied_index == 0 {
            None
        } else {
            Some(openraft::LogId::new_term_index(
                st.last_applied_term,
                st.last_applied_index,
            ))
        };

        let membership = self.last_membership.read().await.clone();
        Ok((last, membership))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where
        Strm: tokio_stream::Stream<
                Item = Result<openraft::storage::EntryResponder<RaftTypeConfig>, io::Error>,
            > + Unpin
            + OptionalSend,
    {
        while let Some(item) = entries.next().await {
            let (entry, responder) = item?;
            let log_id = entry.log_id;
            let term = **log_id.committed_leader_id();

            let response = match entry.payload {
                EntryPayload::Blank => KVResponse::Ok,
                EntryPayload::Membership(membership) => {
                    *self.last_membership.write().await =
                        StoredMembership::new(Some(log_id.clone()), membership);
                    KVResponse::Ok
                }
                EntryPayload::Normal(req) => {
                    let resp = self.apply_request(&req).await.map_err(io::Error::other)?;
                    self.append_log_entry(term, log_id.index, &req)
                        .await
                        .map_err(io::Error::other)?;
                    resp
                }
            };

            self.metadata()
                .save_applied_state(AppliedState {
                    last_applied_index: log_id.index,
                    last_applied_term: term,
                })
                .await
                .map_err(io::Error::other)?;

            if let Some(r) = responder {
                r.send(response);
            }
        }

        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<<RaftTypeConfig as openraft::RaftTypeConfig>::SnapshotData, io::Error> {
        Ok(std::io::Cursor::new(Vec::new()))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<RaftTypeConfig>,
        snapshot: <RaftTypeConfig as openraft::RaftTypeConfig>::SnapshotData,
    ) -> Result<(), io::Error> {
        let incoming = Snapshot {
            meta: meta.clone(),
            snapshot,
        };

        self.install_snapshot_auto(incoming.clone())
            .await
            .map_err(io::Error::other)?;

        if let Some(last) = &meta.last_log_id {
            self.metadata()
                .save_applied_state(AppliedState {
                    last_applied_index: last.index,
                    last_applied_term: **last.committed_leader_id(),
                })
                .await
                .map_err(io::Error::other)?;
        }

        *self.last_membership.write().await = meta.last_membership.clone();
        *self.current_snapshot.write().await = Some(incoming);
        Ok(())
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<RaftTypeConfig>>, io::Error> {
        Ok(self.current_snapshot.read().await.clone())
    }
}

impl RaftLogStorage<RaftTypeConfig> for SurrealStorage {
    type LogReader = SurrealStorage;

    async fn get_log_state(&mut self) -> Result<LogState<RaftTypeConfig>, io::Error> {
        let last_purged = self.last_purged_log_id.read().await.clone();
        let last_log = self
            .raft_entries
            .read()
            .await
            .last_key_value()
            .map(|(_, e)| e.log_id.clone())
            .or_else(|| last_purged.clone());

        Ok(LogState {
            last_purged_log_id: last_purged,
            last_log_id: last_log,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(
        &mut self,
        vote: &openraft::type_config::alias::VoteOf<RaftTypeConfig>,
    ) -> Result<(), io::Error> {
        self.metadata()
            .save_voting_state(VotingState {
                current_term: vote.leader_id.term,
                voted_for: vote.leader_id.voted_for,
            })
            .await
            .map_err(io::Error::other)?;

        let data = postcard::to_stdvec(vote).map_err(io::Error::other)?;
        let mut txn = self.tree().begin().map_err(io::Error::other)?;
        txn.set(b"raft_vote_v2", &data).map_err(io::Error::other)?;
        txn.commit().await.map_err(io::Error::other)?;
        Ok(())
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: IOFlushed<RaftTypeConfig>,
    ) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = <RaftTypeConfig as openraft::RaftTypeConfig>::Entry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let mut logs = self.raft_entries.write().await;

        for entry in entries {
            let term = **entry.log_id.committed_leader_id();
            let index = entry.log_id.index;

            if let EntryPayload::Normal(ref req) = entry.payload {
                self.append_log_entry(term, index, req)
                    .await
                    .map_err(io::Error::other)?;
            }

            logs.insert(index, entry);
        }

        callback.io_completed(Ok(()));
        Ok(())
    }

    async fn truncate_after(
        &mut self,
        last_log_id: Option<openraft::type_config::alias::LogIdOf<RaftTypeConfig>>,
    ) -> Result<(), io::Error> {
        let mut logs = self.raft_entries.write().await;
        match last_log_id {
            Some(last) => logs.retain(|idx, _| *idx <= last.index),
            None => logs.clear(),
        }
        Ok(())
    }

    async fn purge(
        &mut self,
        log_id: openraft::type_config::alias::LogIdOf<RaftTypeConfig>,
    ) -> Result<(), io::Error> {
        let mut logs = self.raft_entries.write().await;
        logs.retain(|idx, _| *idx > log_id.index);
        *self.last_purged_log_id.write().await = Some(log_id);
        Ok(())
    }
}
