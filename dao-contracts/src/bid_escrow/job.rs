//! Bid-related structs.
use casper_dao_utils::{
    casper_dao_macros::{CLTyped, FromBytes, ToBytes},
    casper_env::revert,
    Address,
    BlockTime,
    DocumentHash,
    Error,
};
use casper_types::U512;

use super::types::{BidId, JobId, JobOfferId};
use crate::{
    rules::{
        validation::bid_escrow::{CanPickBid, DoesProposedPaymentMatchTransferred},
        RulesBuilder,
    },
    voting::VotingId,
};

/// Serializable Job status.
#[derive(CLTyped, ToBytes, FromBytes, PartialEq, Eq, Clone, Copy, Debug)]
pub enum JobStatus {
    /// Job
    Created,
    Cancelled,
    /// Job proof submitted.
    Submitted,
    Completed,
}

impl Default for JobStatus {
    fn default() -> Self {
        JobStatus::Created
    }
}

/// Data required to pick the Bid.
pub struct PickBidRequest {
    /// Job id.
    pub job_id: JobId,
    /// Related [`JobOffer`](super::job_offer::JobOffer) id.
    pub job_offer_id: JobOfferId,
    /// Picked Bid id.
    pub bid_id: BidId,
    /// The request creator.
    pub caller: Address,
    /// [JobPoster](crate::bid_escrow#definitions) address.
    pub poster: Address,
    /// [Worker](crate::bid_escrow#definitions) address.
    pub worker: Address,
    /// If the `Worker` is a `VA`.
    pub is_worker_va: bool,
    /// Should be onborded when the Job is done.
    pub onboard: bool,
    /// Time the bid is picked.
    pub block_time: BlockTime,
    /// Time to complete the Job.
    pub timeframe: BlockTime,
    /// Job reward.
    pub payment: U512,
    /// The amount transferred by `Job Poster`.
    pub transferred_cspr: U512,
    /// Bid reputation stake.
    pub stake: U512,
    /// Bid CSPR stake - for an [External Worker](crate::bid_escrow#definitions).
    pub external_worker_cspr_stake: U512,
}

/// Data required to reclaim the Job.
pub struct ReclaimJobRequest {
    /// Job id to update.
    pub new_job_id: JobId,
    /// Bid id to updated.
    pub new_bid_id: BidId,
    /// Time to complete the Job.
    pub proposed_timeframe: BlockTime,
    /// [Worker](crate::bid_escrow#definitions) address.
    pub worker: Address,
    /// Bid reputation stake.
    pub reputation_stake: U512,
    /// Bid CSPR stake - for an [External Worker](crate::bid_escrow#definitions).
    pub cspr_stake: Option<U512>,
    /// Should be onborded when the Job is done.
    pub onboard: bool,
    /// Reclaim time.
    pub block_time: BlockTime,
}

/// Data required to submit job proof.
pub struct SubmitJobProofRequest {
    pub proof: DocumentHash,
}

/// Serializable representation of a `Job`.
#[derive(CLTyped, ToBytes, FromBytes, Debug, Clone)]
pub struct Job {
    job_id: JobId,
    bid_id: BidId,
    job_offer_id: JobOfferId,
    voting_id: Option<VotingId>,
    job_proof: Option<DocumentHash>,
    start_time: BlockTime,
    time_for_job: BlockTime,
    status: JobStatus,
    worker: Address,
    worker_type: WorkerType,
    poster: Address,
    payment: U512,
    stake: U512,
    external_worker_cspr_stake: U512,
    followed_by: Option<JobId>,
}

impl Job {
    /// Conditionally creates a new instance of Job.
    ///
    /// Runs validation:
    /// * [`CanPickBid`]
    /// * [`DoesProposedPaymentMatchTransferred`]
    ///
    /// Stops contract execution if any validation fails.
    pub fn new(request: &PickBidRequest) -> Self {
        RulesBuilder::new()
            .add_validation(CanPickBid::create(request.caller, request.poster))
            .add_validation(DoesProposedPaymentMatchTransferred::create(
                request.payment,
                request.transferred_cspr,
            ))
            .build()
            .validate_generic_validations();

        let worker_type = if request.is_worker_va {
            WorkerType::Internal
        } else if request.onboard {
            WorkerType::ExternalToVA
        } else {
            WorkerType::External
        };

        Job {
            job_id: request.job_id,
            bid_id: request.bid_id,
            job_offer_id: request.job_offer_id,
            voting_id: None,
            job_proof: None,
            start_time: request.block_time,
            time_for_job: request.timeframe,
            status: JobStatus::Created,
            worker: request.worker,
            worker_type,
            poster: request.poster,
            payment: request.payment,
            stake: request.stake,
            external_worker_cspr_stake: request.external_worker_cspr_stake,
            followed_by: None,
        }
    }

    /// Changes the status to [Completed](JobStatus::Completed), creates a new job
    /// with a new `Worker` and `BidId`.
    pub fn reclaim(&mut self, request: ReclaimJobRequest) -> Job {
        self.status = JobStatus::Completed;
        self.followed_by = Some(request.new_job_id);

        let worker_type = match (request.cspr_stake.is_some(), request.onboard) {
            (_, true) => WorkerType::ExternalToVA,
            (true, false) => WorkerType::External,
            (false, false) => WorkerType::Internal,
        };

        Job {
            job_id: request.new_job_id,
            bid_id: request.new_bid_id,
            job_offer_id: self.job_offer_id,
            voting_id: None,
            job_proof: None,
            start_time: request.block_time,
            time_for_job: request.proposed_timeframe,
            status: JobStatus::Submitted,
            worker: request.worker,
            worker_type,
            poster: self.poster,
            payment: self.payment,
            stake: request.reputation_stake,
            external_worker_cspr_stake: request.cspr_stake.unwrap_or_default(),
            followed_by: None,
        }
    }

    /// Verifies if the job can be canceled at a given time.
    ///
    /// # Errors
    /// * [`Error::CannotCancelJob`]
    /// * [`Error::JobCannotBeYetCanceled`]
    pub fn validate_cancel(&self, block_time: BlockTime) -> Result<(), Error> {
        if self.status() != JobStatus::Created {
            return Err(Error::CannotCancelJob);
        }

        if self.finish_time() + self.grace_period() >= block_time {
            return Err(Error::JobCannotBeYetCanceled);
        }

        Ok(())
    }

    /// Changes status to the Cancelled
    pub fn cancel(&mut self, caller: Address) -> Result<(), Error> {
        if self.status() != JobStatus::Created || self.poster() != caller {
            return Err(Error::CannotCancelJob);
        }

        self.status = JobStatus::Cancelled;
        Ok(())
    }

    /// Changes status to the Completed
    pub fn complete(&mut self) {
        self.status = JobStatus::Completed;
    }

    /// Sets a proof and updates the state to [`Submitted`](JobStatus::Submitted).
    ///
    /// # Errors
    /// * [`Error::JobAlreadySubmitted`]
    pub fn submit_proof(&mut self, request: SubmitJobProofRequest) {
        if self.job_proof().is_some() {
            revert(Error::JobAlreadySubmitted);
        }

        self.job_proof = Some(request.proof);
        self.status = JobStatus::Submitted;
    }

    /// Gets the job's status.
    pub fn status(&self) -> JobStatus {
        self.status
    }

    /// Gets the job's worker.
    pub fn worker(&self) -> Address {
        self.worker
    }

    /// Gets the job's poster.    
    pub fn poster(&self) -> Address {
        self.poster
    }

    /// Gets the job's result.
    pub fn result(&self) -> Option<&DocumentHash> {
        self.job_proof.as_ref()
    }

    /// Gets the job's bid id.
    pub fn bid_id(&self) -> BidId {
        self.bid_id
    }

    /// Gets the job's offer id.
    pub fn job_offer_id(&self) -> JobOfferId {
        self.job_offer_id
    }

    /// Gets the job's payment amount.
    pub fn payment(&self) -> U512 {
        self.payment
    }

    /// Gets the job's voting id.
    pub fn voting_id(&self) -> Option<VotingId> {
        self.voting_id
    }

    /// Gets confirmation the job has been done.
    pub fn job_proof(&self) -> Option<&DocumentHash> {
        self.job_proof.as_ref()
    }

    /// Gets the job's finish time.
    pub fn finish_time(&self) -> BlockTime {
        self.start_time + self.time_for_job
    }

    /// Gets the job's worker type.
    pub fn worker_type(&self) -> &WorkerType {
        &self.worker_type
    }

    /// If the worker's vote should be unbound - basically is the reputation real
    pub fn is_unbound(&self) -> bool {
        self.worker_type() != &WorkerType::Internal
    }

    /// Gets the job's stake.
    pub fn get_stake(&self) -> U512 {
        self.stake
    }

    /// Gets the job's CSPR stake.
    pub fn external_worker_cspr_stake(&self) -> U512 {
        self.external_worker_cspr_stake
    }

    /// Links job with [Voting](crate::voting::voting_state_machine::VotingStateMachine).
    pub fn set_voting_id(&mut self, voting_id: VotingId) {
        self.voting_id = Some(voting_id);
    }

    /// Gets the job's CSPR stake.
    pub fn job_id(&self) -> JobId {
        self.job_id
    }

    /// When [Grace Period](crate::bid_escrow#grace-period) starts.
    fn grace_period(&self) -> BlockTime {
        self.time_for_job
    }
}

/// Serializable [Worker](crate::bid_escrow#definitions) type.
#[derive(CLTyped, ToBytes, FromBytes, Debug, PartialEq, Clone)]
pub enum WorkerType {
    /// [VA](crate::bid_escrow#definitions)
    Internal,
    /// Non-VA who becomes a VA once the Job is accepted.
    ExternalToVA,
    /// Non-VA who does not want to become a VA.
    External,
}
