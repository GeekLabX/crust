#![cfg_attr(not(feature = "std"), no_std)]
#![feature(option_result_contains)]

use codec::{Decode, Encode};
use frame_support::{
    decl_event, decl_module, decl_storage, decl_error, dispatch::DispatchResult, ensure,
    traits::{
        Randomness, Currency, ReservableCurrency, LockIdentifier, LockableCurrency,
        WithdrawReasons
    }
};
use sp_std::{prelude::*, convert::TryInto, collections::btree_map::BTreeMap};
use system::ensure_signed;
use sp_runtime::{traits::{StaticLookup, Zero}};

#[cfg(feature = "std")]
use serde::{Deserialize, Serialize};

// Crust runtime modules
use primitives::{
    AddressInfo, MerkleRoot, BlockNumber,
    constants::tee::REPORT_SLOT,
    traits::TransferrableCurrency
};

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

const MARKET_ID: LockIdentifier = *b"market  ";

#[derive(Debug, PartialEq, Eq, Clone, Encode, Decode, Default)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub struct StorageOrder<AccountId, Balance> {
    pub file_identifier: MerkleRoot,
    pub file_size: u64,
    pub created_on: BlockNumber,
    pub completed_on: BlockNumber,
    pub expired_on: BlockNumber,
    pub provider: AccountId,
    pub client: AccountId,
    pub amount: Balance,
    pub status: OrderStatus
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub enum OrderStatus {
    Success,
    Failed,
    Pending
}

#[derive(Debug, PartialEq, Eq, Clone, Encode, Decode, Default)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub struct PledgeLedger<Balance> {
    // total balance of pledge
    pub total: Balance,
    // used balance of pledge
    pub used: Balance
}

impl Default for OrderStatus {
    fn default() -> Self {
        OrderStatus::Pending
    }
}

/// Preference of what happens regarding validation.
#[derive(Debug, PartialEq, Eq, Clone, Encode, Decode, Default)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub struct Provision<Hash> {
    /// Provider's address
    pub address_info: AddressInfo,

    /// Mapping from `file_id` to `order_id`s, this mapping only add when user place the order
    pub file_map: BTreeMap<MerkleRoot, Vec<Hash>>,
}

type BalanceOf<T> =
    <<T as Trait>::Currency as Currency<<T as system::Trait>::AccountId>>::Balance;

/// An event handler for paying market order
pub trait Payment<AccountId, Hash, Balance> {
    /// Reserve client's transferable balances
    fn reserve_sorder(sorder_id: &Hash, client: &AccountId, amount: Balance) -> bool;
    /// Start delayed payment for a reserved storage order
    fn pay_sorder(sorder_id: &Hash);
}

/// A trait for checking order's legality
/// This wanyi is an outer inspector to judge if s/r order can be accepted 😵
pub trait OrderInspector<AccountId> {
    /// Check if the provider can take storage order
    fn check_works(provider: &AccountId, file_size: u64) -> bool;
}

/// Means for interacting with a specialized version of the `market` trait.
///
/// This is needed because `Tee`
/// 1. updates the `Providers` of the `market::Trait`
/// 2. use `Providers` to judge work report
pub trait MarketInterface<AccountId, Hash, Balance> {
    /// Provision{files} will be used for tee module.
    fn providers(account_id: &AccountId) -> Option<Provision<Hash>>;
    /// Vec{order_id} will be used for payment module.
    fn clients(account_id: &AccountId) -> Option<Vec<Hash>>;
    /// Get storage order
    fn maybe_get_sorder(order_id: &Hash) -> Option<StorageOrder<AccountId, Balance>>;
    /// (Maybe) set storage order's status
    fn maybe_set_sorder(order_id: &Hash, so: &StorageOrder<AccountId, Balance>);
}

impl<AId, Hash, Balance> MarketInterface<AId, Hash, Balance> for () {
    fn providers(_: &AId) -> Option<Provision<Hash>> {
        None
    }

    fn clients(_: &AId) -> Option<Vec<Hash>> {
        None
    }

    fn maybe_get_sorder(_: &Hash) -> Option<StorageOrder<AId, Balance>> {
        None
    }

    fn maybe_set_sorder(_: &Hash, _: &StorageOrder<AId, Balance>) {

    }
}

impl<T: Trait> MarketInterface<<T as system::Trait>::AccountId,
    <T as system::Trait>::Hash, BalanceOf<T>> for Module<T>
{
    fn providers(account_id: &<T as system::Trait>::AccountId)
        -> Option<Provision<<T as system::Trait>::Hash>> {
        Self::providers(account_id)
    }

    fn clients(account_id: &<T as system::Trait>::AccountId)
               -> Option<Vec<<T as system::Trait>::Hash>> {
        Self::clients(account_id)
    }

    fn maybe_get_sorder(order_id: &<T as system::Trait>::Hash)
        -> Option<StorageOrder<<T as system::Trait>::AccountId, BalanceOf<T>>> {
        Self::storage_orders(order_id)
    }

    fn maybe_set_sorder(order_id: &<T as system::Trait>::Hash,
                        so: &StorageOrder<<T as system::Trait>::AccountId, BalanceOf<T>>) {
        Self::maybe_set_sorder(order_id, so);
    }
}

/// The module's configuration trait.
pub trait Trait: system::Trait {
    /// The payment balance.
    type Currency: ReservableCurrency<Self::AccountId> + TransferrableCurrency<Self::AccountId>;

    /// The overarching event type.
    type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;

    /// Something that provides randomness in the runtime.
    type Randomness: Randomness<Self::Hash>;

    /// Connector with balance module
    type Payment: Payment<Self::AccountId, Self::Hash, BalanceOf<Self>>;

    /// Connector with tee module
    type OrderInspector: OrderInspector<Self::AccountId>;
}

// This module's storage items.
decl_storage! {
    trait Store for Module<T: Trait> as Market {
        /// A mapping from storage provider to order id
        pub Providers get(fn providers):
        map hasher(twox_64_concat) T::AccountId => Option<Provision<T::Hash>>;

        /// A mapping from clients to order id
        pub Clients get(fn clients):
        map hasher(twox_64_concat) T::AccountId => Option<Vec<T::Hash>>;

        /// Order details iterated by order id
        pub StorageOrders get(fn storage_orders):
        map hasher(twox_64_concat) T::Hash => Option<StorageOrder<T::AccountId, BalanceOf<T>>>;

        /// Pledge details iterated by provider id
        pub PledgeLedgers get(fn pledge_ledgers):
        map hasher(twox_64_concat) T::AccountId => PledgeLedger<BalanceOf<T>>;
    }
}

decl_error! {
    /// Error for the market module.
    pub enum Error for Module<T: Trait> {
        /// Failed on generating order id
        GenerateOrderIdFailed,
        /// No workload
        NoWorkload,
        /// Not provider
        NotProvider,
        /// File duration is too short
        DurationTooShort,
        /// Don't have enough currency
        InsufficientCurrency,
        /// Don't have enough pledge
        InsufficientPledge,
        /// Can not bond with value less than minimum balance.
        InsufficientValue,
        /// Not Pledged before
        NotPledged,
        /// Pledged before
        DoublePledged,
        /// Place order to himself
        PlaceSelfOrder,
    }
}

decl_module! {
    pub struct Module<T: Trait> for enum Call where origin: T::Origin {
        type Error = Error<T>;

        // Initializing events
        // this is needed only if you are using events in your module
        fn deposit_event() = default;

        /// Register to be a provider, you should provide your storage layer's address info
        #[weight = 1_000_000]
        pub fn register(origin, address_info: AddressInfo) -> DispatchResult {
            let who = ensure_signed(origin)?;

            // 1. Make sure you have works
            ensure!(T::OrderInspector::check_works(&who, 0), Error::<T>::NoWorkload);

            // 2. Check if provider has pledged before
            ensure!(<PledgeLedgers<T>>::contains_key(&who), Error::<T>::NotPledged);

            // 3. Upsert provision
            <Providers<T>>::mutate(&who, |maybe_provision| {
                if let Some(provision) = maybe_provision {
                    // Change provider's address info
                    provision.address_info = address_info;
                } else {
                    // New provider
                    *maybe_provision = Some(Provision {
                        address_info,
                        file_map: BTreeMap::new()
                    })
                }
            });

            // 4. Emit success
            Self::deposit_event(RawEvent::RegisterSuccess(who));

            Ok(())
        }

        /// Register to be a provider, you should provide your storage layer's address info
        #[weight = 1_000_000]
        pub fn pledge(
            origin,
            #[compact] value: BalanceOf<T>
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            // 1. Reject a pledge which is considered to be _dust_.
            ensure!(value >= T::Currency::minimum_balance(), Error::<T>::InsufficientValue);

            // 2. Ensure provider has enough currency.
            ensure!(value <= T::Currency::transfer_balance(&who), Error::<T>::InsufficientCurrency);

            // 3. Check if provider has not pledged before
            ensure!(!<PledgeLedgers<T>>::contains_key(&who), Error::<T>::DoublePledged);

            // 4. Prepare new pledge ledger
            let pledge_ledger = PledgeLedger {
                total: value,
                used: Zero::zero()
            };

            // 5 Upsert pledge ledger
            Self::upsert_pledge_ledger(&who, &pledge_ledger);

            // 6. Emit success
            Self::deposit_event(RawEvent::PledgeSuccess(who));

            Ok(())
        }

        /// Pledge extra amount of currency to accept market order.
        #[weight = 1_000_000]
        pub fn pledge_extra(origin, #[compact] value: BalanceOf<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;

            // 1. Reject a pledge which is considered to be _dust_.
            ensure!(value >= T::Currency::minimum_balance(), Error::<T>::InsufficientValue);

            // 2. Check if provider has pledged before
            ensure!(<PledgeLedgers<T>>::contains_key(&who), Error::<T>::NotPledged);

            // 3. Ensure provider has enough currency.
            ensure!(value <= T::Currency::transfer_balance(&who), Error::<T>::InsufficientCurrency);

            let mut pledge_ledger = Self::pledge_ledgers(&who);
            // 4. Increase total value
            pledge_ledger.total += value;

            // 5 Upsert pledge ledger
            Self::upsert_pledge_ledger(&who, &pledge_ledger);

            // 6. Emit success
            Self::deposit_event(RawEvent::PledgeSuccess(who));

            Ok(())
        }

        /// Decrease pledge amount of currency for market order.
        #[weight = 1_000_000]
        pub fn cut_pledge(origin, #[compact] value: BalanceOf<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;

            // 1. Reject a pledge which is considered to be _dust_.
            ensure!(value >= T::Currency::minimum_balance(), Error::<T>::InsufficientValue);

            // 2. Check if provider has pledged before
            ensure!(<PledgeLedgers<T>>::contains_key(&who), Error::<T>::NotPledged);

            // 3. Ensure value is smaller than unused.
            let mut pledge_ledger = Self::pledge_ledgers(&who);
            ensure!(value <= pledge_ledger.total - pledge_ledger.used, Error::<T>::InsufficientPledge);

            // 4. Decrease total value
            pledge_ledger.total -= value;

            // 5 Upsert pledge ledger
            if pledge_ledger.total.is_zero() {
                <PledgeLedgers<T>>::remove(&who);
                // remove the lock.
                T::Currency::remove_lock(MARKET_ID, &who);
            } else {
                Self::upsert_pledge_ledger(&who, &pledge_ledger);
            }

            // 6. Emit success
            Self::deposit_event(RawEvent::PledgeSuccess(who));

            Ok(())
        }

        /// Place a storage order
        #[weight = 1_000_000]
        pub fn place_storage_order(
            origin,
            provider: <T::Lookup as StaticLookup>::Source,
            #[compact] amount: BalanceOf<T>,
            file_identifier: MerkleRoot,
            file_size: u64,
            duration: u32
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let provider = T::Lookup::lookup(provider)?;

            // 1. Cannot place storage order to himself.
            ensure!(who != provider, Error::<T>::PlaceSelfOrder);

            // 2. Expired should be greater than created
            ensure!(duration > REPORT_SLOT.try_into().unwrap(), Error::<T>::DurationTooShort);

            // 3. Check if provider is registered
            ensure!(<Providers<T>>::contains_key(&provider), Error::<T>::NotProvider);

            // 4. Check provider has capacity to store file
            ensure!(T::OrderInspector::check_works(&provider, file_size), Error::<T>::NoWorkload);

            // 5. Check client has enough currency to pay
            ensure!(T::Currency::can_reserve(&who, amount.clone()), Error::<T>::InsufficientCurrency);

            // 6. Check if provider pledged
            ensure!(<PledgeLedgers<T>>::contains_key(&provider), Error::<T>::InsufficientPledge);

            // 7. Check provider has unused pledge
            let pledge_ledger = Self::pledge_ledgers(&provider);
            ensure!(amount <= pledge_ledger.total - pledge_ledger.used, Error::<T>::InsufficientPledge);

            // 8. Construct storage order
            let created_on = TryInto::<u32>::try_into(<system::Module<T>>::block_number()).ok().unwrap();
            let storage_order = StorageOrder::<T::AccountId, BalanceOf<T>> {
                file_identifier,
                file_size,
                created_on,
                completed_on: created_on,
                expired_on: created_on + duration, // this will be changed, when `status` become `Success`
                provider: provider.clone(),
                client: who.clone(),
                amount,
                status: OrderStatus::Pending
            };

            // 9. Pay the order and (maybe) add storage order
            if Self::maybe_insert_sorder(&who, &provider, &storage_order) {
                // a. update ledger
                <PledgeLedgers<T>>::mutate(&provider, |pledge_ledger| {
                        pledge_ledger.used += amount;
                });
                // b. emit storage order success event
                Self::deposit_event(RawEvent::StorageOrderSuccess(who, storage_order));
            } else {
                // c. emit error
                Err(Error::<T>::GenerateOrderIdFailed)?
            }

            Ok(())
        }
    }
}

impl<T: Trait> Module<T> {
    // MUTABLE PUBLIC
    pub fn maybe_set_sorder(order_id: &T::Hash, so: &StorageOrder<T::AccountId, BalanceOf<T>>) {
        if let Some(old_sorder) = Self::storage_orders(order_id) {
            if &old_sorder != so {
                // 1. Order has been confirmed in the first time { Pending -> Success }
                if old_sorder.status == OrderStatus::Pending &&
                    so.status == OrderStatus::Success {
                    T::Payment::pay_sorder(order_id);
                }

                // TODO: add market slashing here
                // TODO: Record `failed_count` from `Success` to `Failed`
                // TODO: Record `failed_duration` from `Failed` to `Success`

                // 2. Update storage order
                <StorageOrders<T>>::insert(order_id, so);
            }
        }
    }

    // MUTABLE PRIVATE
    // `sorder` is equal to storage order
    fn maybe_insert_sorder(client: &T::AccountId,
                           provider: &T::AccountId,
                           so: &StorageOrder<T::AccountId, BalanceOf<T>>) -> bool {
        let order_id = Self::get_sorder_id(client, provider);

        // This should be false, cause we don't allow duplicated `order_id`
        if <StorageOrders<T>>::contains_key(&order_id) {
            false
        } else {
            // 0. If reserve client's balance failed return error
            // TODO: return different error type
            if !T::Payment::reserve_sorder(&order_id, client, so.amount) {
                return false
            }

            // 1. Add new storage order
            <StorageOrders<T>>::insert(&order_id, so);

            // 2. Add `order_id` to client orders
            <Clients<T>>::mutate(client, |maybe_client_orders| {
                if let Some(client_order) = maybe_client_orders {
                    client_order.push(order_id.clone());
                } else {
                    *maybe_client_orders = Some(vec![order_id.clone()])
                }
            });

            // 3. Add `file_identifier` -> `order_id`s to provider's file_map
            <Providers<T>>::mutate(provider, |maybe_provision| {
                // `provision` cannot be None
                if let Some(provision) = maybe_provision {
                    let mut order_ids: Vec::<T::Hash> = vec![];
                    if let Some(o_ids) = provision.file_map.get(&so.file_identifier) {
                        order_ids = o_ids.clone();
                    }

                    order_ids.push(order_id);
                    provision.file_map.insert(so.file_identifier.clone(), order_ids.clone());
                }
            });

            true
        }
    }

    fn get_sorder_id(client: &T::AccountId, provider: &T::AccountId) -> T::Hash {
        // 1. Construct random seed
        // seed = [ block_hash, client_account, provider_account ]
        let bn = <system::Module<T>>::block_number();
        let bh: T::Hash = <system::Module<T>>::block_hash(bn);
        let seed = [
            &bh.as_ref()[..],
            &client.encode()[..],
            &provider.encode()[..],
        ].concat();

        // 2. It can cover most cases, for the "real" random
        T::Randomness::random(seed.as_slice())
    }

    fn upsert_pledge_ledger(
        provider: &T::AccountId,
        pledge_ledger: &PledgeLedger<BalanceOf<T>>
    ) {
        // 1. Set lock
        T::Currency::set_lock(
            MARKET_ID,
            &provider,
            pledge_ledger.total,
            WithdrawReasons::all(),
        );
        // 2. Update PledgeLedger
        <PledgeLedgers<T>>::insert(&provider, pledge_ledger);
    }
}

decl_event!(
    pub enum Event<T>
    where
        AccountId = <T as system::Trait>::AccountId,
        Balance = BalanceOf<T>
    {
        StorageOrderSuccess(AccountId, StorageOrder<AccountId, Balance>),
        RegisterSuccess(AccountId),
        PledgeSuccess(AccountId),
    }
);
