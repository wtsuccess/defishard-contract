use crate::*;

/// approval callbacks from NFT Contracts

const INITIAL_BALANCE: Balance = 2_000_000_000_000_000_000_000_000;

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct SaleArgs {
    pub sale_conditions: SaleConditions,
    pub token_type: TokenType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_auction: Option<bool>,
}

trait NonFungibleTokenApprovalsReceiver {
    fn nft_on_approve(
        &mut self,
        token_id: TokenId,
        owner_id: ValidAccountId,
        approval_id: u64,
        msg: String,
    );
}

#[near_bindgen]
impl NonFungibleTokenApprovalsReceiver for Contract {
    /// where we add the sale because we know nft owner can only call nft_approve

    fn nft_on_approve(
        &mut self,
        token_id: TokenId,
        owner_id: ValidAccountId,
        approval_id: u64,
        msg: String,
    ) {
        // enforce cross contract call and owner_id is signer

        let nft_contract_id = env::predecessor_account_id();
        let signer_id = env::signer_account_id();
        assert!(vec!["dev-1695922751973-50100434093733"].contains(&nft_contract_id.as_str()), "nft_contract_id is not whitelisted");
        assert_ne!(
            nft_contract_id,
            signer_id,
            "nft_on_approve should only be called via cross-contract call"
        );
        assert_eq!(
            owner_id.as_ref(),
            &signer_id,
            "owner_id should be signer_id"
        );

        let SaleArgs { mut sale_conditions, token_type, is_auction } =
            near_sdk::serde_json::from_str(&msg).expect("Not valid SaleArgs");

        for (ft_token_id, mut price) in sale_conditions.clone() {
            if !self.ft_token_ids.contains(&ft_token_id) {
                env::panic(
                    format!("Token {} not supported by this market", ft_token_id).as_bytes(),
                );
            }

            price = U128(price.0 + u128::from(INITIAL_BALANCE));
            sale_conditions.remove(&ft_token_id);
            sale_conditions.insert(ft_token_id, price);
        }

        let bids = HashMap::new();

        let contract_and_token_id = format!("{}{}{}", nft_contract_id, DELIMETER, token_id);
        self.sales.insert(
            &contract_and_token_id,
            &Sale {
                owner_id: owner_id.clone().into(),
                approval_id,
                nft_contract_id: nft_contract_id.clone(),
                token_id: token_id.clone(),
                sale_conditions,
                bids,
                created_at: U64(env::block_timestamp()/1000000),
                token_type: token_type.clone(),
                is_auction: is_auction.unwrap_or(false),
            },
        );

        // extra for views

        let mut by_owner_id = self.by_owner_id.get(owner_id.as_ref()).unwrap_or_else(|| {
            UnorderedSet::new(
                StorageKey::ByOwnerIdInner {
                    account_id_hash: hash_account_id(owner_id.as_ref()),
                }
                .try_to_vec()
                .unwrap(),
            )
        });

        by_owner_id.insert(&contract_and_token_id);
        self.by_owner_id.insert(owner_id.as_ref(), &by_owner_id);

        let mut by_nft_contract_id = self
            .by_nft_contract_id
            .get(&nft_contract_id)
            .unwrap_or_else(|| {
                UnorderedSet::new(
                    StorageKey::ByNFTContractIdInner {
                        account_id_hash: hash_account_id(&nft_contract_id),
                    }
                    .try_to_vec()
                    .unwrap(),
                )
            });
        by_nft_contract_id.insert(&token_id);
        self.by_nft_contract_id
            .insert(&nft_contract_id, &by_nft_contract_id);

        if let Some(token_type) = token_type {
            assert!(token_id.contains(&token_type), "TokenType should be substr of TokenId");
            let mut by_nft_token_type = self
                .by_nft_token_type
                .get(&token_type)
                .unwrap_or_else(|| {
                    UnorderedSet::new(
                        StorageKey::ByNFTTokenTypeInner {
                            token_type_hash: hash_account_id(&token_type),
                        }
                        .try_to_vec()
                        .unwrap(),
                    )
                });
                by_nft_token_type.insert(&contract_and_token_id);
            self.by_nft_token_type
                .insert(&token_type, &by_nft_token_type);
        }

        let current_user = near_sdk::env::current_account_id();
        ext_contract::nft_transfer(
            current_user.clone(),
            token_id,
            approval_id,
            "deposit to market".to_string(),
            &nft_contract_id,
            1,
            GAS_FOR_NFT_TRANSFER,
        );
    }
}
