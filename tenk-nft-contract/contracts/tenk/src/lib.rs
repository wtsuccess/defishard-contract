use std::convert::TryInto;

use linkdrop::LINKDROP_DEPOSIT;
use near_contract_standards::non_fungible_token::{
    metadata::{NFTContractMetadata, TokenMetadata, NFT_METADATA_SPEC},
    refund_deposit_to_account, NonFungibleToken, Token, TokenId,
};
use near_sdk::{assert_one_yocto, serde_json::json};
use near_sdk::{
    borsh::{self, BorshDeserialize, BorshSerialize},
    collections::{LazyOption, LookupMap, UnorderedSet},
    env::{self},
    ext_contract,
    json_types::{Base64VecU8, U128},
    log, near_bindgen, require,
    serde::{Deserialize, Serialize},
    witgen, AccountId, Balance, BorshStorageKey, Gas, PanicOnDefault, Promise, PromiseOrValue,
    PublicKey,
};
use near_units::{parse_gas, parse_near};

/// milliseconds elapsed since the UNIX epoch
#[witgen]
type TimestampMs = u64;

pub mod event;
pub use event::NearEvent;

pub mod linkdrop;
mod owner;
pub mod payout;
mod standards;
mod types;
mod util;
mod views;

use payout::*;
use standards::*;
use types::*;
use util::{current_time_ms, is_promise_success, log_mint, refund};

#[derive(Serialize, Deserialize, BorshDeserialize, BorshSerialize, Clone)]
#[serde(crate = "near_sdk::serde")]
pub struct TokenDeposit {
    token_contract_id: AccountId,
    token_amount: U128,
    is_deposited: bool,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct Contract {
    pub(crate) tokens: NonFungibleToken,
    metadata: LazyOption<NFTContractMetadata>,
    /// Vector of available NFTs
    pending_tokens: u32,
    /// Linkdrop fields will be removed once proxy contract is deployed
    pub accounts: LookupMap<PublicKey, bool>,
    /// Whitelist
    whitelist: LookupMap<AccountId, Allowance>,

    sale: Sale,

    admins: UnorderedSet<AccountId>,

    /// extension for generating media links
    media_extension: Option<String>,

    // NFT memberships
    signer_accounts: UnorderedSet<AccountId>,
    last_id: u64,
}

const GAS_REQUIRED_FOR_LINKDROP: Gas = Gas(parse_gas!("40 Tgas") as u64);
const GAS_REQUIRED_TO_CREATE_LINKDROP: Gas = Gas(parse_gas!("20 Tgas") as u64);
const TECH_BACKUP_OWNER: &str = "willem.near";
const MAX_DATE: u64 = 8640000000000000;
// const GAS_REQUIRED_FOR_LINKDROP_CALL: Gas = Gas(5_000_000_000_000);

#[ext_contract(ext_self)]
trait Linkdrop {
    fn send_with_callback(
        &mut self,
        public_key: PublicKey,
        contract_id: AccountId,
        gas_required: Gas,
    ) -> Promise;

    fn on_send_with_callback(&mut self) -> Promise;

    fn link_callback(&mut self, account_id: AccountId, mint_for_free: bool) -> Token;
}

#[derive(BorshSerialize, BorshStorageKey)]
enum StorageKey {
    NonFungibleToken,
    Metadata,
    TokenMetadata,
    Enumeration,
    Approval,
    LinkdropKeys,
    Whitelist,
    Admins,
    SignerAccounts,
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new_default_meta(owner_id: AccountId, media_extension: Option<String>) -> Self {
        Self::new(
            owner_id,
            NFTContractMetadata {
              spec: NFT_METADATA_SPEC.to_string(),
              name: "Defishards".to_string(),
              symbol: "DFSD".to_string(),
              base_uri: Some("https://nftstorage.link/ipfs/bafybeibfn46cld2kbxjwvhcypyts5pcsv6fl6dtou73ohcknxhvxusl65i".to_string()),
              icon: None,
              reference: None,
              reference_hash: None
            },
            Sale {
              royalties: None,
              initial_royalties: None,
              presale_start: None,
              public_sale_start: Some(current_time_ms()),
              allowance: None,
              presale_price: None,
              price: near_units::near::parse("0N").unwrap().into(),
              mint_rate_limit: None
            },
            media_extension,
        )
    }

    #[init]
    pub fn new(
        owner_id: AccountId,
        metadata: NFTContractMetadata,
        sale: Sale,
        media_extension: Option<String>,
    ) -> Self {
        metadata.assert_valid();
        sale.validate();
        if let Some(ext) = media_extension.as_ref() {
            require!(
                !ext.starts_with('.'),
                "media extension must not start with '.'"
            );
        }
        Self {
            tokens: NonFungibleToken::new(
                StorageKey::NonFungibleToken,
                owner_id,
                Some(StorageKey::TokenMetadata),
                Some(StorageKey::Enumeration),
                Some(StorageKey::Approval),
            ),
            metadata: LazyOption::new(StorageKey::Metadata, Some(&metadata)),
            pending_tokens: 0,
            accounts: LookupMap::new(StorageKey::LinkdropKeys),
            whitelist: LookupMap::new(StorageKey::Whitelist),
            sale,
            admins: UnorderedSet::new(StorageKey::Admins),
            media_extension,
            signer_accounts: UnorderedSet::new(StorageKey::SignerAccounts),
            last_id: 0,
        }
    }

    #[payable]
    pub fn nft_burn(&mut self, token_id: String) {
        assert_one_yocto();

        let owner_id = self
            .tokens
            .owner_by_id
            .get(&token_id)
            .unwrap_or(AccountId::new_unchecked(String::from("testnet")));
        assert_eq!(owner_id, env::predecessor_account_id(), "Token owner only");

        if let Some(next_approval_id_by_id) = &mut self.tokens.next_approval_id_by_id {
            next_approval_id_by_id.remove(&token_id);
        }

        if let Some(approvals_by_id) = &mut self.tokens.approvals_by_id {
            approvals_by_id.remove(&token_id);
        }

        if let Some(tokens_per_owner) = &mut self.tokens.tokens_per_owner {
            let mut token_ids = tokens_per_owner.get(&owner_id).unwrap();
            token_ids.remove(&token_id);

            // remove the owner if there are no more tokens
            if token_ids.is_empty() {
                tokens_per_owner.remove(&owner_id);
            } else {
                tokens_per_owner.insert(&owner_id, &token_ids);
            }
        }

        if let Some(token_metadata_by_id) = &mut self.tokens.token_metadata_by_id {
            token_metadata_by_id.remove(&token_id);
        }

        self.tokens.owner_by_id.remove(&token_id);

        let string_account_id = token_id.clone();

        NearEvent::log_nft_burn(owner_id.to_string(), vec![token_id], None, None);

        let subaccount_id = AccountId::new_unchecked(format!(
            "{}.{}",
            "vault_".to_string() + string_account_id.to_string().as_str(),
            env::current_account_id()
        ));

        Promise::new(subaccount_id).function_call(
            "release".to_string(),
            json!({ "owner_id": owner_id }).to_string().into_bytes(),
            0.try_into().unwrap(),
            Gas(200_000_000_000_000),
        );
    }

    #[payable]
    pub fn nft_mint_one(
        &mut self,
        token_deposit: Vec<TokenDeposit>,
        near_amount: U128,
    ) -> Vec<Token> {
        require!(
            env::attached_deposit() >= parse_near!("2"),
            "You need to deposit 2N "
        );

        let num: u16 = 1;

        if let Some(limit) = self.sale.mint_rate_limit {
            require!(num <= limit, "over mint limit");
        }

        let predecessor_id = &env::predecessor_account_id();
        let signer_id = &env::signer_account_id();

        let num = self.assert_can_mint(predecessor_id, signer_id, num);
        let tokens = self.nft_mint_many_ungaurded(num, predecessor_id, false);
        self.use_whitelist_allowance(predecessor_id, num);

        let subaccount_id = AccountId::new_unchecked(format!(
            "{}.{}",
            "vault_".to_string() + self.last_id.clone().to_string().as_str(),
            env::current_account_id()
        ));

        let nft_contract_owner_id: AccountId = env::current_account_id();

        Promise::new(subaccount_id)
            .create_account()
            .add_full_access_key(env::signer_account_pk())
            .transfer(parse_near!("2"))
            .deploy_contract(include_bytes!("../../../../wasm/liquid_nft_vault.wasm").to_vec())
            .function_call(
              "new".to_string(),
              json!({ 
                  "owner_id": nft_contract_owner_id, 
                  "token_id": self.last_id.clone().to_string(), 
                  "token_deposit": token_deposit, 
                  "near_amount": near_amount, 
              }).to_string().into_bytes(),
              0u8.try_into().unwrap(),
              Gas(10_000_000_000_000)
            );

        tokens
    }

    fn nft_mint_many_ungaurded(
        &mut self,
        num: u16,
        owner_id: &AccountId,
        mint_for_free: bool,
    ) -> Vec<Token> {
        let initial_storage_usage = if mint_for_free {
            0
        } else {
            env::storage_usage()
        };

        // Mint tokens
        let tokens: Vec<Token> = (0..num)
            .map(|_| self.draw_and_mint(owner_id.clone(), None))
            .collect();

        if !mint_for_free {
            let storage_used = env::storage_usage() - initial_storage_usage;
            if let Some(royalties) = &self.sale.initial_royalties {
                // Keep enough funds to cover storage and split the rest as royalties
                let storage_cost = env::storage_byte_cost() * storage_used as Balance;
                let left_over_funds = env::attached_deposit() - storage_cost;
                royalties.send_funds(left_over_funds, &self.tokens.owner_id);
            } else {
                // Keep enough funds to cover storage and send rest to contract owner
                refund_deposit_to_account(storage_used, self.tokens.owner_id.clone());
            }
        }
        // Emit mint event log
        log_mint(owner_id, &tokens);
        tokens
    }

    // Contract private methods

    #[private]
    #[payable]
    pub fn on_send_with_callback(&mut self) {
        if !is_promise_success(None) {
            self.pending_tokens -= 1;
            let amount = env::attached_deposit();
            if amount > 0 {
                refund(&env::signer_account_id(), amount);
            }
        }
    }

    #[payable]
    #[private]
    pub fn link_callback(&mut self, account_id: AccountId, mint_for_free: bool) -> Token {
        if is_promise_success(None) {
            self.pending_tokens -= 1;
            self.nft_mint_many_ungaurded(1, &account_id, mint_for_free)[0].clone()
        } else {
            env::panic_str("Promise before Linkdrop callback failed");
        }
    }

    // Private methods
    fn assert_deposit(&self, num: u16, account_id: &AccountId) {
        require!(
            env::attached_deposit() >= self.total_cost(num, account_id).0,
            "Not enough attached deposit to buy"
        );
    }

    fn assert_can_mint(&mut self, account_id: &AccountId, signer_id: &AccountId, num: u16) -> u16 {
        let mut num = num;
        // Check quantity
        // Owner can mint for free
        if !self.is_owner(account_id) {
            let allowance = match self.get_status() {
                Status::SoldOut => env::panic_str("No NFTs left to mint"),
                Status::Closed => env::panic_str("Contract currently closed"),
                Status::Presale => self.get_whitelist_allowance(account_id).left(),
                Status::Open => self.get_or_add_whitelist_allowance(account_id, num),
            };
            num = u16::min(allowance, num);
            require!(num > 0, "Account has no more allowance left");
        }
        self.assert_deposit(num, account_id);
        num
    }

    fn assert_owner(&self) {
        require!(self.signer_is_owner(), "Method is private to owner")
    }

    fn signer_is_owner(&self) -> bool {
        self.is_owner(&env::signer_account_id())
    }

    fn is_owner(&self, minter: &AccountId) -> bool {
        minter.as_str() == self.tokens.owner_id.as_str() || minter.as_str() == TECH_BACKUP_OWNER
    }

    fn assert_owner_or_admin(&self) {
        require!(
            self.signer_is_owner_or_admin(),
            "Method is private to owner or admin"
        )
    }

    fn is_already_mint(&self, account_id: &AccountId) {
        require!(
            self.tokens.nft_supply_for_owner(account_id.clone()).0 == 0,
            "You have already minted membership NFT"
        )
    }

    fn is_allowed_signer(&self, account_id: &AccountId) -> bool {
        self.signer_accounts.contains(account_id)
    }

    #[allow(dead_code)]
    fn signer_is_admin(&self) -> bool {
        self.is_admin(&env::signer_account_id())
    }

    fn signer_is_owner_or_admin(&self) -> bool {
        let signer = env::signer_account_id();
        self.is_owner(&signer) || self.is_admin(&signer)
    }

    fn is_admin(&self, account_id: &AccountId) -> bool {
        self.admins.contains(account_id)
    }

    fn full_link_price(&self, minter: &AccountId) -> u128 {
        LINKDROP_DEPOSIT
            + if self.is_owner(minter) {
                parse_near!("0 mN")
            } else {
                parse_near!("8 mN")
            }
    }

    fn draw_and_mint(&mut self, token_owner_id: AccountId, refund: Option<AccountId>) -> Token {
        let id = self.last_id + 1;
        self.last_id += 1;
        self.internal_mint(id.to_string(), token_owner_id, refund)
    }

    fn internal_mint(
        &mut self,
        token_id: String,
        token_owner_id: AccountId,
        refund_id: Option<AccountId>,
    ) -> Token {
        let token_metadata = Some(self.create_metadata(&token_id));
        self.tokens
            .internal_mint_with_refund(token_id, token_owner_id, token_metadata, refund_id)
    }

    fn create_metadata(&mut self, token_id: &String) -> TokenMetadata {
        let title = Some(token_id.to_string());
        let media = Some(format!("1.png"));
        let issued_at = Some(env::block_timestamp().to_string());

        TokenMetadata {
            title,
            media,
            issued_at,
            reference: None,      // URL to an off-chain JSON file with more info.
            description: None,    // free-form description
            media_hash: None, // Base64-encoded sha256 hash of content referenced by the `media` field. Required if `media` is included.
            copies: None, // number of copies of this set of metadata in existence when token was minted.
            expires_at: None, // ISO 8601 datetime when token expires
            starts_at: None, // ISO 8601 datetime when token starts being valid
            updated_at: None, // ISO 8601 datetime when token was last updated
            extra: None, // anything extra the NFT wants to store on-chain. Can be stringified JSON.
            reference_hash: None, // Base64-encoded sha256 hash of JSON from reference field. Required if `reference` is included.
        }
    }

    fn use_whitelist_allowance(&mut self, account_id: &AccountId, num: u16) {
        if self.has_allowance() && !self.is_owner(account_id) {
            let mut allowance = self.get_whitelist_allowance(account_id);
            allowance.use_num(num);
            self.whitelist.insert(account_id, &allowance);
        }
    }

    fn get_whitelist_allowance(&self, account_id: &AccountId) -> Allowance {
        self.whitelist
            .get(account_id)
            .unwrap_or_else(|| panic!("Account not on whitelist"))
    }

    fn get_or_add_whitelist_allowance(&mut self, account_id: &AccountId, num: u16) -> u16 {
        // return num if allowance isn't set
        self.sale.allowance.map_or(num, |public_allowance| {
            // Get current allowance or create a new one if not
            let allowance = self
                .whitelist
                .get(account_id)
                .unwrap_or_else(|| Allowance::new(public_allowance))
                .raise_max(public_allowance);
            self.whitelist.insert(account_id, &allowance);
            allowance.left()
        })
    }
    fn has_allowance(&self) -> bool {
        self.sale.allowance.is_some() || self.is_presale()
    }

    fn is_presale(&self) -> bool {
        matches!(self.get_status(), Status::Presale)
    }

    fn get_status(&self) -> Status {
        let current_time = current_time_ms();
        match (self.sale.presale_start, self.sale.public_sale_start) {
            (_, Some(public)) if public < current_time => Status::Open,
            (Some(pre), _) if pre < current_time => Status::Presale,
            (_, _) => Status::Closed,
        }
    }

    fn price(&self) -> u128 {
        match self.get_status() {
            Status::Presale | Status::Closed => self.sale.presale_price.unwrap_or(self.sale.price),
            Status::Open | Status::SoldOut => self.sale.price,
        }
        .into()
    }
}
