use near_contract_standards::fungible_token::receiver::FungibleTokenReceiver;
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::json_types::U128;
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::serde_json::json;
use near_sdk::{
    env, near_bindgen, require, AccountId, Balance, Gas, PanicOnDefault, Promise, PromiseOrValue,
};

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct AssetArgs {
    token_id: String,
    near_amount: U128,
    near_deposited: bool,
    token_deposit: Vec<TokenDeposit>,
}

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
    owner_id: AccountId,
    token_id: String,
    near_amount: U128,
    near_deposited: bool,
    token_deposit: Vec<TokenDeposit>,
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new(
        owner_id: AccountId,
        token_id: String,
        near_amount: U128,
        token_deposit: Vec<TokenDeposit>,
    ) -> Self {
        require!(!env::state_exists(), "Already initialized");

        for token in &token_deposit {
            require!(
                env::is_valid_account_id(token.token_contract_id.as_bytes()),
                "Not valid token contract id"
            );
            require!(token.token_amount > U128(0), "Cannot wrap 0 token");
            require!(token.is_deposited == true, "is_deposit must be true");
        }

        Self {
            owner_id,
            token_id,
            near_amount,
            near_deposited: false,
            token_deposit,
        }
    }

    pub fn get_info(&self) -> AssetArgs {
        AssetArgs {
            token_id: String::from(self.token_id.clone()),
            near_amount: self.near_amount,
            near_deposited: self.near_deposited,
            token_deposit: self.token_deposit.clone(),
        }
    }

    pub fn release(&mut self, owner_id: AccountId) {
        assert_eq!(env::predecessor_account_id(), self.owner_id, "Unauthorized");

        if self.near_deposited {
            Promise::new(owner_id.clone()).transfer(u128::from(self.near_amount));
            self.near_deposited = false;
        }

        for token in self.token_deposit.clone() {
            if token.is_deposited {
                Promise::new(token.token_contract_id).function_call(
                    "ft_transfer".to_string(),
                    json!({
                      "receiver_id": owner_id.clone(), "amount": token.token_amount
                    })
                    .to_string()
                    .into_bytes(),
                    1.try_into().unwrap(),
                    Gas(60_000_000_000_000),
                );
            }
        }

        Promise::new(env::current_account_id()).delete_account(owner_id);
    }

    #[payable]
    pub fn deposit_near(&mut self) {
        require!(
            self.near_amount != U128(0)
                && !self.near_deposited
                && u128::from(self.near_amount)
                    .checked_div(100)
                    .unwrap()
                    .checked_add(u128::from(self.near_amount))
                    .unwrap()
                    == env::attached_deposit(),
            "Can not accept Near Deposit"
        );
        Promise::new(self.owner_id.clone())
            .transfer(u128::from(self.near_amount).checked_div(100).unwrap());
        self.near_deposited = true;
    }
}

#[near_bindgen]
impl FungibleTokenReceiver for Contract {
    /// Called by fungible token contract after `ft_transfer_call` was initiated by
    /// `sender_id` of the given `amount` with the transfer message given in `msg` field.
    /// The `amount` of tokens were already transferred to this contract account and ready to be used.
    ///
    /// The method must return the amount of tokens that are *not* used/accepted by this contract from the transferred
    /// amount. Examples:
    /// - The transferred amount was `500`, the contract completely takes it and must return `0`.
    /// - The transferred amount was `500`, but this transfer call only needs `450` for the action passed in the `msg`
    ///   field, then the method must return `50`.
    /// - The transferred amount was `500`, but the action in `msg` field has expired and the transfer must be
    ///   cancelled. The method must return `500` or panic.
    ///
    /// Arguments:
    /// - `sender_id` - the account ID that initiated the transfer.
    /// - `amount` - the amount of tokens that were transferred to this account in a decimal string representation.
    /// - `msg` - a string message that was passed with this transfer call.
    ///
    /// Returns the amount of unused tokens that should be returned to sender, in a decimal string representation.
    fn ft_on_transfer(
        &mut self,
        _sender_id: AccountId,
        amount: U128,
        _msg: String,
    ) -> PromiseOrValue<U128> {
        let token_contract_id = env::predecessor_account_id();

        for token in &mut self.token_deposit {
            if token.token_contract_id == token_contract_id {
                let require_amount = token.token_amount;
                if token.is_deposited == false
                    && u128::from(require_amount)
                        .checked_div(100)
                        .unwrap()
                        .checked_add(u128::from(
                            u128::from(self.near_amount)
                                .checked_div(100)
                                .unwrap()
                                .checked_add(u128::from(self.near_amount))
                                .unwrap(),
                        ))
                        .unwrap()
                        == u128::from(amount)
                {
                    Promise::new(token.token_contract_id.clone())
              .function_call("ft_transfer".to_string(),
              json!({ "receiver_id": self.owner_id.clone(), "amount": U128(u128::from(require_amount).checked_div(100).unwrap())}).to_string().into_bytes(),
              1.try_into().unwrap(),
              Gas(60_000_000_000_000));
                    token.is_deposited = true
                } else {
                    return PromiseOrValue::Value(amount);
                }
            }
        }

        PromiseOrValue::Value(U128(0))
    }
}
