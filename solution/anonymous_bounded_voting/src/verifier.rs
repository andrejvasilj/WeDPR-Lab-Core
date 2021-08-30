// Copyright 2020 WeDPR Lab Project Authors. Licensed under Apache-2.0.

//! Library of anonymous bounded voting (ABV) solution.

use wedpr_l_utils::error::WedprError;
use wedpr_s_protos::generated::abv::{
    Ballot, CandidateBallot, CountingPart, DecryptedResultPartStorage,
    VoteResultStorage, VoteStorage,
};

use crate::config::{HASH_KECCAK256, SIGNATURE_SECP256K1};
use curve25519_dalek::{ristretto::RistrettoPoint, scalar::Scalar};
use wedpr_l_crypto_zkp_discrete_logarithm_proof::{
    verify_equality_relationship_proof, verify_format_proof,
    verify_sum_relationship,
};
use wedpr_l_crypto_zkp_range_proof::verify_value_range_in_batch;
use wedpr_l_crypto_zkp_utils::{
    bytes_to_point, point_to_bytes, BASEPOINT_G1, BASEPOINT_G2,
};
use wedpr_l_protos::{
    bytes_to_proto,
    generated::zkp::{BalanceProof, EqualityProof},
};
use wedpr_l_utils::traits::{Hash, Signature};
use wedpr_s_protos::generated::abv::{SystemParametersStorage, VoteRequest};

pub fn verify_bounded_vote_request(
    param: &SystemParametersStorage,
    request: &VoteRequest,
    public_key: &[u8],
) -> Result<bool, WedprError> {
    let poll_point = bytes_to_point(param.get_poll_point())?;
    let signature = request.get_vote().get_signature();
    let blank_ballot = request.get_vote().get_blank_ballot();
    let mut hash_vec = Vec::new();
    hash_vec.append(&mut blank_ballot.get_ciphertext1().to_vec());
    hash_vec.append(&mut blank_ballot.get_ciphertext2().to_vec());
    let message_hash: Vec<u8> = HASH_KECCAK256.hash(&hash_vec);

    if !SIGNATURE_SECP256K1.verify(
        &public_key,
        &message_hash.as_ref(),
        &signature,
    ) {
        return Err(WedprError::VerificationError);
    }

    let range_proof = request.get_range_proof();
    let mut commitments: Vec<RistrettoPoint> = Vec::new();
    let mut voted_ballot_sum = RistrettoPoint::default();
    for candidate_ballot_pair in request.get_vote().get_voted_ballot() {
        let ballot = candidate_ballot_pair.get_ballot();
        commitments.push(bytes_to_point(&ballot.get_ciphertext1())?);
        voted_ballot_sum += bytes_to_point(&ballot.get_ciphertext1())?;
    }

    let rest_ballot = request.get_vote().get_rest_ballot().get_ciphertext1();
    let rest_ballot_point = bytes_to_point(rest_ballot.clone())?;
    commitments.push(rest_ballot_point);
    pending_commitment_vec(&mut commitments);
    if !verify_value_range_in_batch(&commitments, range_proof, &poll_point) {
        wedpr_println!("verify range proof failed!");
        return Err(WedprError::VerificationError);
    }

    for candidate_ballot in request.get_ballot_proof() {
        let candidate = candidate_ballot.get_candidate();
        let ballot_proof = candidate_ballot.get_value();
        let mut candidate_ballot = Ballot::new();
        for candidate_ballot_pair in request.get_vote().get_voted_ballot() {
            if candidate_ballot_pair.get_candidate() == candidate {
                candidate_ballot = candidate_ballot_pair.get_ballot().clone();
            }
        }

        let ciphertext1 = bytes_to_point(&candidate_ballot.get_ciphertext1())?;
        let ciphertext2 = bytes_to_point(&candidate_ballot.get_ciphertext2())?;
        let format_proof_bytes = ballot_proof.get_format_proof();
        let format_proof = bytes_to_proto::<BalanceProof>(format_proof_bytes)?;
        if !verify_format_proof(
            &ciphertext1,
            &ciphertext2,
            &format_proof,
            &*BASEPOINT_G1,
            &*BASEPOINT_G2,
            &poll_point,
        )? {
            wedpr_println!("verify_format failed!");
            return Err(WedprError::VerificationError);
        }
    }
    let balance_proof_bytes = request.get_sum_balance_proof();
    let balance_proof = bytes_to_proto::<BalanceProof>(balance_proof_bytes)?;
    if !verify_sum_relationship(
        &voted_ballot_sum,
        &bytes_to_point(&rest_ballot)?,
        &bytes_to_point(&blank_ballot.get_ciphertext1())?,
        &balance_proof,
        &BASEPOINT_G1,
        &poll_point,
    )? {
        wedpr_println!("verify_balance failed!");
        return Err(WedprError::VerificationError);
    }
    Ok(true)
}

pub fn aggregate_vote_sum_response(
    param: &SystemParametersStorage,
    vote_storage_part: &VoteStorage,
    vote_sum: &mut VoteStorage,
) -> Result<bool, WedprError> {
    if !vote_sum.has_blank_ballot() {
        vote_sum
            .mut_blank_ballot()
            .set_ciphertext1(point_to_bytes(&RistrettoPoint::default()));
        vote_sum
            .mut_blank_ballot()
            .set_ciphertext2(point_to_bytes(&RistrettoPoint::default()));
        for candidate in param.get_candidates().get_candidate() {
            let mut ballot = Ballot::new();
            ballot.set_ciphertext1(point_to_bytes(&RistrettoPoint::default()));
            ballot.set_ciphertext2(point_to_bytes(&RistrettoPoint::default()));
            let mut ballot_pair = CandidateBallot::new();
            ballot_pair.set_candidate(candidate.to_string());
            ballot_pair.set_ballot(ballot);
            vote_sum.mut_voted_ballot().push(ballot_pair);
        }
    }

    let mut tmp_vote_storage_sum = VoteStorage::new();
    let mut blank_c1_sum =
        bytes_to_point(&vote_sum.get_blank_ballot().get_ciphertext1())?;
    let mut blank_c2_sum =
        bytes_to_point(&vote_sum.get_blank_ballot().get_ciphertext2())?;
    let c1_tmp_point = bytes_to_point(
        &vote_storage_part
            .get_blank_ballot()
            .get_ciphertext1()
            .clone(),
    )?;
    let c2_tmp_point = bytes_to_point(
        &vote_storage_part
            .get_blank_ballot()
            .get_ciphertext2()
            .clone(),
    )?;
    blank_c1_sum += c1_tmp_point;
    blank_c2_sum += c2_tmp_point;

    for candidate in param.get_candidates().get_candidate() {
        let mut candidate_ballot = Ballot::new();
        for tmp_pair in vote_sum.get_voted_ballot() {
            if tmp_pair.get_candidate() == candidate {
                candidate_ballot = tmp_pair.get_ballot().clone();
            }
        }
        let mut candidate_voted_c1_sum =
            bytes_to_point(&candidate_ballot.get_ciphertext1())?;
        let mut candidate_voted_c2_sum =
            bytes_to_point(&candidate_ballot.get_ciphertext2())?;
        let mut candidates_ballot = Ballot::new();
        for ballot_pair in vote_storage_part.get_voted_ballot() {
            if candidate == ballot_pair.get_candidate() {
                candidates_ballot = ballot_pair.get_ballot().clone();
            }
        }
        candidate_voted_c1_sum +=
            bytes_to_point(&candidates_ballot.get_ciphertext1())?;
        candidate_voted_c2_sum +=
            bytes_to_point(&candidates_ballot.get_ciphertext2())?;
        let mut vote_ballot = Ballot::new();
        vote_ballot.set_ciphertext1(point_to_bytes(&candidate_voted_c1_sum));
        vote_ballot.set_ciphertext2(point_to_bytes(&candidate_voted_c2_sum));
        let mut tmp_pair = CandidateBallot::new();
        tmp_pair.set_candidate(candidate.to_string());
        tmp_pair.set_ballot(vote_ballot);
        tmp_vote_storage_sum.mut_voted_ballot().push(tmp_pair);
    }
    tmp_vote_storage_sum
        .mut_blank_ballot()
        .set_ciphertext1(point_to_bytes(&blank_c1_sum));
    tmp_vote_storage_sum
        .mut_blank_ballot()
        .set_ciphertext2(point_to_bytes(&blank_c2_sum));
    *vote_sum = tmp_vote_storage_sum.clone();
    Ok(true)
}

pub fn verify_count_request(
    param: &SystemParametersStorage,
    encrypted_vote_sum: &VoteStorage,
    counter_share: &RistrettoPoint,
    request: &DecryptedResultPartStorage,
) -> Result<bool, WedprError> {
    let blank_c2_sum = bytes_to_point(
        &encrypted_vote_sum.get_blank_ballot().get_ciphertext2(),
    )?;
    let blank_equality_proof_bytes =
        request.get_blank_part().get_equality_proof();
    let blank_c2_r = bytes_to_point(&request.get_blank_part().get_c2_r())?;
    let blank_equality_proof =
        bytes_to_proto::<EqualityProof>(&blank_equality_proof_bytes)?;
    if !verify_equality_relationship_proof(
        &counter_share,
        &blank_c2_r,
        &blank_equality_proof,
        &BASEPOINT_G2,
        &blank_c2_sum,
    )? {
        return Ok(false);
    }
    for candidate in param.get_candidates().get_candidate() {
        let mut candidate_ballot = Ballot::new();
        for pair in encrypted_vote_sum.get_voted_ballot() {
            if candidate == pair.get_candidate() {
                candidate_ballot = pair.get_ballot().clone();
            }
        }
        let candidate_c2_sum =
            bytes_to_point(&candidate_ballot.get_ciphertext2())?;
        let mut counting_part = CountingPart::new();
        for pair in request.get_candidate_part() {
            if candidate == pair.get_key() {
                counting_part = pair.get_value().clone();
            }
        }
        let candidate_c2_r = bytes_to_point(&counting_part.get_c2_r())?;
        let candidate_equality_proof_bytes = counting_part.get_equality_proof();
        let candidate_equality_proof =
            bytes_to_proto::<EqualityProof>(candidate_equality_proof_bytes)?;

        if !verify_equality_relationship_proof(
            &counter_share,
            &candidate_c2_r,
            &candidate_equality_proof.clone(),
            &BASEPOINT_G2,
            &candidate_c2_sum,
        )? {
            wedpr_println!("verify_equality failed!");
            return Ok(false);
        }
    }
    Ok(true)
}

// In this function, everyone can check anonymousvoting result by c1 - c2r_sum,
// because we already know v and candidates, by using c1 - c2r_sum, we can check
// whether vG_1 =? c1 - c2r_sum. pub fn verify_counter(result_pb:
pub fn verify_vote_result(
    param: &SystemParametersStorage,
    vote_sum: &VoteStorage,
    counting_result_sum: &DecryptedResultPartStorage,
    vote_result_request: &VoteResultStorage,
) -> Result<bool, WedprError> {
    let blank_c1_sum =
        bytes_to_point(&vote_sum.get_blank_ballot().get_ciphertext1())?;
    let blank_c2_r_sum =
        bytes_to_point(&counting_result_sum.get_blank_part().get_c2_r())?;
    let expected_blank_ballot_result = blank_c1_sum - (blank_c2_r_sum);
    let mut get_blank_result: i64 = 0;
    for tmp in vote_result_request.get_result() {
        if tmp.get_key() == "Wedpr_voting_total_ballots" {
            get_blank_result = tmp.get_value();
        }
    }

    if expected_blank_ballot_result
        .ne(&(*BASEPOINT_G1 * (Scalar::from(get_blank_result as u64))))
    {
        wedpr_println!("verify blank_ballot_result failed!");
        return Ok(false);
    }

    for candidate in param.get_candidates().get_candidate() {
        let mut ballot = Ballot::new();
        for tmp_pair in vote_sum.get_voted_ballot() {
            if candidate == tmp_pair.get_candidate() {
                ballot = tmp_pair.get_ballot().clone();
            }
        }

        let mut candidate_counting_part = CountingPart::new();
        for tmp_pair in counting_result_sum.get_candidate_part() {
            if candidate == tmp_pair.get_key() {
                candidate_counting_part = tmp_pair.get_value().clone();
            }
        }

        let candidate_c2_r_sum =
            bytes_to_point(&candidate_counting_part.get_c2_r())?;

        let expected_candidate_ballot_result =
            bytes_to_point(&ballot.get_ciphertext1())? - (candidate_c2_r_sum);

        let mut get_candidate_result: i64 = 0;
        for tmp in vote_result_request.get_result() {
            if tmp.get_key() == candidate {
                get_candidate_result = tmp.get_value();
            }
        }
        if !expected_candidate_ballot_result
            .eq(&(*BASEPOINT_G1 * (Scalar::from(get_candidate_result as u64))))
        {
            wedpr_println!("verify candidate {} failed!", candidate);
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn pending_commitment_vec(v: &mut Vec<RistrettoPoint>) {
    let length = v.len() as i32;
    let log_length = (length as f64).log2().ceil() as u32;
    let expected_len = 2_i32.pow(log_length);
    if expected_len == length {
        return;
    }
    let pending_length = expected_len - length;
    let tmp = RistrettoPoint::default();
    for _ in 0..pending_length {
        v.push(tmp);
    }
}
