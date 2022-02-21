#![no_std]

elrond_wasm::imports!();
elrond_wasm::derive_imports!();

#[derive(Debug, TopDecode, TopEncode, NestedDecode, NestedEncode, TypeAbi)]
pub struct PoolInfo<M: ManagedTypeApi> {
    acc_reward_per_share: BigUint<M>,
    last_reward_time: u64,
    alloc_point: BigUint<M>,
}

#[derive(Debug, TopDecode, TopEncode, NestedDecode, NestedEncode, TypeAbi)]
pub struct UserInfo<M: ManagedTypeApi> {
    amount: BigUint<M>,
    reward_debt: BigUint<M>,
}

const ACC_REWARD_PRECISION: u64 = 10u64.pow(12);

/// One of the simplest smart contracts possible,
/// it holds a single variable in storage, which anyone can increment.
#[elrond_wasm::derive::contract]
pub trait MasterChef {
    #[view(getSum)]
    #[storage_mapper("sum")]
    fn sum(&self) -> SingleValueMapper<BigInt>;

    #[init]
    fn init(&self, reward_token: &TokenIdentifier) {
        // not required
        self.reward_token().set(reward_token);
    }

    // ====== ENDPOINTS ======

    /// Add a new LP to the pool. Can only be called by the owner.
    /// DO NOT add the same LP token more than once. Rewards will be messed up if you do.
    #[endpoint]
    #[only_owner]
    fn add(&self, want: TokenIdentifier, alloc_point: &BigUint) {
        // TODO: check duplicate, mass update pool
        // require!(!self.want_tokens().contains(&want), "alreadyAdded");
        let pool_id = self.want_tokens().len();
        self.want_tokens().push(&want);
        let info = PoolInfo {
            alloc_point: alloc_point.clone(),
            last_reward_time: self.blockchain().get_block_nonce(),
            acc_reward_per_share: BigUint::zero(),
        };
        self.pool_info(pool_id).set(&info);
        self.total_alloc_point().update(|x| *x += alloc_point);
    }

    /// Update the given pool's reward allocation point
    #[endpoint]
    fn set(&self, pool_id: usize, alloc_point: &BigUint) {
        let pool = self.pool_info(pool_id).get();
        let current_alloc_point = pool.alloc_point;
        self.pool_info(pool_id).update(|x| {
            x.alloc_point = alloc_point.clone();
        });
        self.total_alloc_point()
            .update(|x| *x = x.clone() + alloc_point - &current_alloc_point);

        self.emit_pool_updated_event(pool_id, &alloc_point);
    }

    /// Deposit LP tokens to MCV2 for reward allocation.
    #[endpoint]
    #[payable("*")]
    fn deposit(&self, pool_id: usize) {
        let (amount, token_identifier) = self.call_value().payment_token_pair();
        let want_token = self.want_tokens().get(pool_id);
        require!(token_identifier == want_token, "invalid token");
        self.update_pool(pool_id);
        let pool = self.pool_info(pool_id).get();
        let caller = self.blockchain().get_caller();
        self.user_info(&caller).update(|user| {
            user.amount += &amount;
            user.reward_debt += &amount * &pool.acc_reward_per_share / ACC_REWARD_PRECISION;
        });
        self.emit_deposited_event(&caller, pool_id, &amount);
    }

    /// Deposit LP tokens to MCV2 for reward allocation.
    #[endpoint]
    fn withdraw(&self, pool_id: usize, amount: &BigUint) {
        self.update_pool(pool_id);

        let want_token = self.want_tokens().get(pool_id);
        let caller = self.blockchain().get_caller();
        let pool = self.pool_info(pool_id).get();

        // effect
        self.user_info(&caller).update(|user| {
            user.amount -= amount;
            user.reward_debt -= amount * &pool.acc_reward_per_share / ACC_REWARD_PRECISION;
        });

        self.send().direct(&caller, &want_token, 0, amount, &[]);
        self.emit_withdrawn_event(&caller, pool_id, amount);
    }

    #[endpoint]
    fn harvest(&self, pool_id: usize) {
        self.update_pool(pool_id);
        let pool = self.pool_info(pool_id).get();
        let caller = self.blockchain().get_caller();
        let user = self.user_info(&caller).get();
        let acc_reward = user.amount * pool.acc_reward_per_share / ACC_REWARD_PRECISION;
        let pending_reward = acc_reward - user.reward_debt;

        let reward_token = self.reward_token().get();
        self.send().direct(&caller, &reward_token, 0, &pending_reward, &[]);
    }

    // ======= private ======
    fn update_pool(&self, id: usize) {
        let pool = self.pool_info(id).get();
        let want_token = self.want_tokens().get(id);
        let current_time = self.blockchain().get_block_nonce();

        if current_time > pool.last_reward_time {
            let lp_supply = self.blockchain().get_sc_balance(&want_token, 0);

            self.pool_info(id).update(|pool| {
                let reward_per_second = self.reward_per_block().get();
                let total_alloc_point = self.total_alloc_point().get();

                if lp_supply > 0 {
                    let time = current_time - pool.last_reward_time;
                    let reward_amount =
                        (BigUint::from(time) * &reward_per_second * &pool.alloc_point)
                            / &total_alloc_point;
                    pool.acc_reward_per_share += (reward_amount * ACC_REWARD_PRECISION) / lp_supply;
                }
                pool.last_reward_time = current_time;
            })
        }
    }

    // events
    #[event("deposited")]
    fn emit_deposited_event(
        &self,
        #[indexed] from: &ManagedAddress,
        #[indexed] pool_id: usize,
        amount: &BigUint,
    );
    
    #[event("withdrawn")]
    fn emit_withdrawn_event(
        &self,
        #[indexed] from: &ManagedAddress,
        #[indexed] pool_id: usize,
        amount: &BigUint,
    );

    #[event("pool_added")]
    fn emit_pool_added_event(
        &self,
        #[indexed] token: &TokenIdentifier,
        #[indexed] pool_id: usize,
        alloc_point: &BigUint,
    );

    #[event("pool_updated")]
    fn emit_pool_updated_event(&self, #[indexed] pool_id: usize, alloc_point: &BigUint);

    // storage
    #[view]
    #[storage_mapper("pool_info")]
    fn pool_info(&self, pool_id: usize) -> SingleValueMapper<PoolInfo<Self::Api>>;

    #[view]
    #[storage_mapper("user_info")]
    fn user_info(&self, user: &ManagedAddress) -> SingleValueMapper<UserInfo<Self::Api>>;

    #[view]
    #[storage_mapper("reward_token")]
    fn reward_token(&self) -> SingleValueMapper<TokenIdentifier>;

    #[view]
    #[storage_mapper("reward_per_block")]
    fn reward_per_block(&self) -> SingleValueMapper<BigUint>;

    #[view]
    #[storage_mapper("total_alloc_point")]
    fn total_alloc_point(&self) -> SingleValueMapper<BigUint>;

    /// store list of want tokens in added order
    #[view]
    #[storage_mapper("want_tokens")]
    fn want_tokens(&self) -> VecMapper<TokenIdentifier>;
}
