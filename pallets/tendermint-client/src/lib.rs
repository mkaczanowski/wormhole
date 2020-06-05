#![cfg_attr(not(feature = "std"), no_std)]

/// Wormhole TendermintClient Pallet. Allows verification of Tendermint block headers on the substrate chain.

use frame_support::{decl_module, decl_storage, decl_event, decl_error, dispatch, ensure};
use frame_system::{self as system, ensure_signed};

use tendermint_light_client::{
	LightSignedHeader, LightValidatorSet, verify_commit_full, verify_single, TrustedState, TrustThresholdFraction,
};
use chrono::{Utc};
use core::time::Duration;
use sha2::{Sha256, Digest};

extern crate alloc;
extern crate core;
extern crate std;
use sp_std::vec::Vec;
use log::{error, debug};

mod types;

use crate::types::{TendermintClient, ConsensusState, TMCreateClientPayload, TMUpdateClientPayload, TMClientStorageWrapper};

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

/// The pallet's configuration trait.
pub trait Trait: system::Trait {
	// Add other types and constants required to configure this pallet.

	/// The overarching event type.
	type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
}

// This pallet's storage items.
decl_storage! {
	// It is important to update your storage name so that your pallet's
	// storage items are isolated from other pallets.
	// ---------------------------------vvvvvvvvvvvvvv
	trait Store for Module<T: Trait> as TendermintClientModule {

		TMClientStorage: map hasher(blake2_128_concat) Vec<u8> => TMClientStorageWrapper;
	}
}

// The pallet's events
decl_event!(
	pub enum Event<T> where AccountId = <T as system::Trait>::AccountId {
		/// Just a dummy event.
		/// Event `ClientCreated`/`ClientUpdated` is declared with a parameter of the type `string` (name), `string` (chainid), `u64` (height)
		/// To emit this event, we call the deposit function, from our runtime functions
		ClientCreated(AccountId, Vec<u8>, Vec<u8>, u64),
		ClientUpdated(AccountId, Vec<u8>, Vec<u8>, u64),
	}
);

// The pallet's errors
decl_error! {
	pub enum Error for Module<T: Trait> {
		/// Value was None
		NoneValue,
		/// Value reached maximum and cannot be incremented further
		StorageOverflow,
		/// Item not found in storage.
		ItemNotFound,
		/// Unable to deserialize extrinsic.
		DeserializeError,
		/// Parsing Error occurred
		ParseError,
		/// Error occurred validating block.
		ValidationError,
	}
}

// The pallet's dispatchable functions.
decl_module! {
	/// The module declaration.
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		// Initializing errors
		// this includes information about your errors in the node's metadata.
		// it is needed only if you are using errors in your pallet
		type Error = Error<T>;

		// Initializing events
		// this is needed only if you are using events in your pallet
		fn deposit_event() = default;

		/// Client initialisation entry point.
		/// takes tendermint/ibc/tendermint/CreateClient message.
		#[weight = 100_000]
		pub fn init_client(origin, payload: Vec<u8>) -> dispatch::DispatchResult {
			debug!("init client");
			debug!("Submitted payload: {:?}", &payload[..]);
			// Check it was signed and get the signer. See also: ensure_root and ensure_none
			let who = ensure_signed(origin)?;
			let r: Result<_, _> = serde_json::from_slice(&payload[..]);
			let container: TMCreateClientPayload = r.map_err(|e| {
			  error!("Deserialization Error: {}", e);
			  Error::<T>::DeserializeError
			})?;
			let header: LightSignedHeader = container.header.signed_header;
			let trust_period: u64 = container.trusting_period;
			let max_clock_drift: u64 = container.max_clock_drift;
			let unbonding_period: u64 = container.unbonding_period;

			tendermint_light_client::verify_commit_full(&container.validator_set, &header).map_err(|e| {
			  error!("Validation Error: {}", e);
			  Error::<T>::ValidationError
			});

			// TODO:  validate client name
			// TODO:  validate trust_period
			let state: ConsensusState = ConsensusState{
				state: 	TrustedState::new(header, container.validator_set),
				last_update: Utc::now(),
			};

			let tmclient: TendermintClient = TendermintClient{
				state: Some(state.clone()),
				trusting_period: trust_period,
				client_id: container.client_id.clone(),
				max_clock_drift: max_clock_drift,
				unbonding_period: unbonding_period,
				chain_id: header.header.chain_id.as_bytes().to_vec(),
				trust_threshold: TrustThresholdFraction::default(),
			};

			let mut hasher = Sha256::new();
			hasher.input(&container.client_id);
			let key = hasher.result();

			TMClientStorage::insert(key.as_slice(), TMClientStorageWrapper{client: tmclient.clone()});
			// TODO: does this error if the key already exists?

			// Here we are raising the ClientCreated event
			Self::deposit_event(RawEvent::ClientCreated(who, tmclient.client_id, tmclient.chain_id, header.header.height.value()));
			Ok(())
		}

		/// Update client entry point.
		#[weight = 100_000]
		pub fn update_client(origin, payload: Vec<u8>) -> dispatch::DispatchResult {

			// Check it was signed and get the signer. See also: ensure_root and ensure_none
			let _who = ensure_signed(origin)?;

			let r: Result<_, _> = serde_json::from_slice(&payload[..]);
			let container: TMUpdateClientPayload = r.map_err(|e| {
			  error!("Deserialization Error: {}", e);
			  Error::<T>::DeserializeError
			})?;

			let mut hasher = Sha256::new();
			hasher.input(&container.client_id);
			let key = hasher.result();

			ensure!(TMClientStorage::contains_key(key.as_slice()), Error::<T>::ItemNotFound);

            let mut wrapped_client: TMClientStorageWrapper = TMClientStorage::get(key.as_slice());

			let header: LightSignedHeader = container.header.signed_header;

			let trusted_state = tendermint_light_client::verify_single(
				wrapped_client.client.state.unwrap().state.clone(),
				&header,
				&container.validator_set,
				&container.next_validator_set,
				wrapped_client.client.trust_threshold,
				Duration::from_secs(wrapped_client.client.trusting_period+wrapped_client.client.max_clock_drift),
				std::time::SystemTime::now(),
			).map_err(|e| {
				error!("Unable to validate header: {}", e);
				Error::<T>::ValidationError
			})?;

			// TODO:  validate header
			let state: ConsensusState = ConsensusState{
				state: trusted_state,
				last_update: Utc::now(),
			};
			wrapped_client.client.state = Some(state.clone());
			TMClientStorage::insert(key.as_slice(), wrapped_client.clone());
			Ok(())
		}
	}
}
