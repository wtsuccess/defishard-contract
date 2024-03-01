use crate::*;

#[near_bindgen]
impl Contract {
    /// Current contract owner
    pub fn owner(&self) -> AccountId {
        self.tokens.owner_id.clone()
    }

    /// Current set of admins
    pub fn admins(&self) -> Vec<AccountId> {
        self.admins.to_vec()
    }

    /// Check whether an account is allowed to mint during the presale
    pub fn whitelisted(&self, account_id: &AccountId) -> bool {
        self.whitelist.contains_key(account_id)
    }

    /// Cost of NFT + fees for linkdrop
    pub fn cost_of_linkdrop(&self, minter: &AccountId) -> U128 {
        (self.full_link_price(minter) + self.total_cost(1, minter).0 + self.token_storage_cost().0)
            .into()
    }

    pub fn total_cost(&self, num: u16, minter: &AccountId) -> U128 {
        (num as Balance * self.cost_per_token(minter).0).into()
    }

    /// Flat cost of one token
    pub fn cost_per_token(&self, minter: &AccountId) -> U128 {
        if self.is_owner(minter) {
            0
        } else {
            self.price()
        }
        .into()
    }

    /// Current cost in NEAR to store one NFT
    pub fn token_storage_cost(&self) -> U128 {
        (env::storage_byte_cost() * self.tokens.extra_storage_in_bytes_per_token as Balance).into()
    }

    /// Part of the NFT metadata standard. Returns the contract's metadata
    pub fn nft_metadata(&self) -> NFTContractMetadata {
        self.metadata.get().unwrap()
    }

    /// How many tokens an account is still allowed to mint. None, means unlimited
    pub fn remaining_allowance(&self, account_id: &AccountId) -> Option<u16> {
        let allowance = if self.is_presale() {
            0
        } else if let Some(allowance) = self.sale.allowance {
            allowance
        } else {
            return None;
        };
        self.whitelist
            .get(account_id)
            .map(|a| a.raise_max(allowance).left())
    }

    /// Max number of mints in one transaction. None, means unlimited
    pub fn mint_rate_limit(&self) -> Option<u16> {
        self.sale.mint_rate_limit
    }
}
