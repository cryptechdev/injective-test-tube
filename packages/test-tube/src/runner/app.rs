use std::ffi::CString;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use cosmrs::crypto::secp256k1::SigningKey;
use cosmrs::proto::tendermint::v0_38::abci::ResponseFinalizeBlock;
use cosmrs::tx;
use cosmrs::tx::{Fee, SignerInfo};
use cosmwasm_std::Coin;
use prost::Message;

use crate::account::{Account, FeeSetting, SigningAccount};
use crate::bindings::{
    AccountNumber, AccountSequence, FinalizeBlock, GetBlockHeight, GetBlockTime, GetParamSet,
    GetValidatorAddress, GetValidatorPrivateKey, IncreaseTime, InitAccount, InitTestEnv, Query,
    Simulate,
};
use crate::redefine_as_go_string;
use crate::runner::error::{DecodeError, EncodeError, RunnerError};
use crate::runner::result::RawResult;
use crate::runner::result::{RunnerExecuteResult, RunnerResult};
use crate::runner::Runner;

pub const INJECTIVE_MIN_GAS_PRICE: u128 = 2_500;

#[derive(Debug, PartialEq)]
pub struct BaseApp {
    id: u64,
    fee_denom: String,
    chain_id: String,
    address_prefix: String,
    default_gas_adjustment: f64,
}

impl BaseApp {
    pub fn new(
        fee_denom: &str,
        chain_id: &str,
        address_prefix: &str,
        default_gas_adjustment: f64,
    ) -> Self {
        let id = unsafe { InitTestEnv() };
        BaseApp {
            id,
            fee_denom: fee_denom.to_string(),
            chain_id: chain_id.to_string(),
            address_prefix: address_prefix.to_string(),
            default_gas_adjustment,
        }
    }

    /// Increase the time of the blockchain by the given number of seconds.
    pub fn increase_time(&self, seconds: u64) {
        unsafe {
            IncreaseTime(self.id, seconds.try_into().unwrap());
        }
    }

    /// Get the first validator address
    pub fn get_first_validator_address(&self) -> RunnerResult<String> {
        let addr = unsafe {
            let addr = GetValidatorAddress(self.id, 0);
            CString::from_raw(addr)
        }
        .to_str()
        .map_err(DecodeError::Utf8Error)?
        .to_string();

        Ok(addr)
    }

    /// Get the first validator private key
    pub fn get_first_validator_private_key(&self) -> RunnerResult<String> {
        let pkey = unsafe {
            let pkey = GetValidatorPrivateKey(self.id, 0);
            CString::from_raw(pkey)
        }
        .to_str()
        .map_err(DecodeError::Utf8Error)?
        .to_string();

        Ok(pkey)
    }

    /// Get the first validator signing account
    pub fn get_first_validator_signing_account(
        &self,
        denom: String,
        gas_adjustment: f64,
    ) -> RunnerResult<SigningAccount> {
        let pkey = unsafe {
            let pkey = GetValidatorPrivateKey(self.id, 0);
            CString::from_raw(pkey)
        }
        .to_str()
        .map_err(DecodeError::Utf8Error)?
        .to_string();

        println!("pkey: {:?}", pkey);

        let secp256k1_priv = BASE64_STANDARD
            .decode(pkey)
            .map_err(DecodeError::Base64DecodeError)?;

        let signing_key = SigningKey::from_slice(&secp256k1_priv).unwrap();

        let validator = SigningAccount::new(
            "inj".to_string(),
            signing_key,
            FeeSetting::Auto {
                gas_price: Coin::new(INJECTIVE_MIN_GAS_PRICE, denom),
                gas_adjustment,
            },
        );

        Ok(validator)
    }

    /// Get the current block time
    pub fn get_block_time_nanos(&self) -> i64 {
        unsafe { GetBlockTime(self.id) }
    }

    /// Get the current block height
    pub fn get_block_height(&self) -> i64 {
        unsafe { GetBlockHeight(self.id) }
    }
    /// Initialize account with initial balance of any coins.
    /// This function mints new coins and send to newly created account
    pub fn init_account(&self, coins: &[Coin]) -> RunnerResult<SigningAccount> {
        let mut coins = coins.to_vec();

        // invalid coins if denom are unsorted
        coins.sort_by(|a, b| a.denom.cmp(&b.denom));

        let coins_json = serde_json::to_string(&coins).map_err(EncodeError::JsonEncodeError)?;
        redefine_as_go_string!(coins_json);

        let empty_tx = "".to_string();
        redefine_as_go_string!(empty_tx);

        let base64_priv = unsafe {
            let addr = InitAccount(self.id, coins_json);
            FinalizeBlock(self.id, empty_tx);
            CString::from_raw(addr)
        }
        .to_str()
        .map_err(DecodeError::Utf8Error)?
        .to_string();

        let secp256k1_priv = BASE64_STANDARD
            .decode(base64_priv)
            .map_err(DecodeError::Base64DecodeError)?;

        let signing_key = SigningKey::from_slice(&secp256k1_priv).map_err(|e| {
            let msg = e.to_string();
            DecodeError::SigningKeyDecodeError { msg }
        })?;

        Ok(SigningAccount::new(
            self.address_prefix.clone(),
            signing_key,
            FeeSetting::Auto {
                gas_price: Coin::new(INJECTIVE_MIN_GAS_PRICE, self.fee_denom.clone()),
                gas_adjustment: self.default_gas_adjustment,
            },
        ))
    }
    /// Convenience function to create multiple accounts with the same
    /// Initial coins balance
    pub fn init_accounts(&self, coins: &[Coin], count: u64) -> RunnerResult<Vec<SigningAccount>> {
        (0..count).map(|_| self.init_account(coins)).collect()
    }

    fn create_signed_tx<I>(
        &self,
        msgs: I,
        signer: &SigningAccount,
        fee: Fee,
    ) -> RunnerResult<Vec<u8>>
    where
        I: IntoIterator<Item = cosmrs::Any>,
    {
        let tx_body = tx::Body::new(msgs, "", 0u32);
        let addr = signer.address();

        redefine_as_go_string!(addr);

        let seq = unsafe { AccountSequence(self.id, addr) };

        let account_number = unsafe { AccountNumber(self.id, addr) };
        let signer_info = SignerInfo::single_direct(Some(signer.public_key()), seq);
        let auth_info = signer_info.auth_info(fee);
        let sign_doc = tx::SignDoc::new(
            &tx_body,
            &auth_info,
            &(self
                .chain_id
                .parse()
                .expect("parse const str of chain id should never fail")),
            account_number,
        )
        .map_err(|e| match e.downcast::<prost::EncodeError>() {
            Ok(encode_err) => EncodeError::ProtoEncodeError(encode_err),
            Err(e) => panic!("expect `prost::EncodeError` but got {:?}", e),
        })?;

        let tx_raw = sign_doc.sign(signer.signing_key()).unwrap();

        tx_raw
            .to_bytes()
            .map_err(|e| match e.downcast::<prost::EncodeError>() {
                Ok(encode_err) => EncodeError::ProtoEncodeError(encode_err),
                Err(e) => panic!("expect `prost::EncodeError` but got {:?}", e),
            })
            .map_err(RunnerError::EncodeError)
    }

    pub fn simulate_tx<I>(
        &self,
        msgs: I,
        signer: &SigningAccount,
    ) -> RunnerResult<cosmrs::proto::cosmos::base::abci::v1beta1::GasInfo>
    where
        I: IntoIterator<Item = cosmrs::Any>,
    {
        let zero_fee = Fee::from_amount_and_gas(
            cosmrs::Coin {
                denom: self.fee_denom.parse().unwrap(),
                amount: INJECTIVE_MIN_GAS_PRICE,
            },
            0u64,
        );

        let tx = self.create_signed_tx(msgs, signer, zero_fee)?;
        let base64_tx_bytes = BASE64_STANDARD.encode(tx);

        redefine_as_go_string!(base64_tx_bytes);

        unsafe {
            let res = Simulate(self.id, base64_tx_bytes);
            let res = RawResult::from_non_null_ptr(res).into_result()?;

            cosmrs::proto::cosmos::base::abci::v1beta1::GasInfo::decode(res.as_slice())
                .map_err(DecodeError::ProtoDecodeError)
                .map_err(RunnerError::DecodeError)
        }
    }
    fn estimate_fee<I>(&self, msgs: I, signer: &SigningAccount) -> RunnerResult<Fee>
    where
        I: IntoIterator<Item = cosmrs::Any>,
    {
        let res = match &signer.fee_setting() {
            FeeSetting::Auto {
                gas_price,
                gas_adjustment,
            } => {
                let gas_info = self.simulate_tx(msgs, signer)?;
                let gas_limit = ((gas_info.gas_used as f64) * (gas_adjustment)).ceil() as u64;

                let amount = cosmrs::Coin {
                    denom: self.fee_denom.parse().unwrap(),
                    amount: (((gas_limit as f64) * (gas_price.amount.u128() as f64)).ceil() as u64)
                        .into(),
                };
                Ok(Fee::from_amount_and_gas(amount, gas_limit))
            }
            FeeSetting::Custom { .. } => {
                panic!("estimate fee is a private function and should never be called when fee_setting is Custom");
            }
        };

        res
    }

    /// Get parameter set for a given subspace.
    pub fn get_param_set<P: Message + Default>(
        &self,
        subspace: &str,
        type_url: &str,
    ) -> RunnerResult<P> {
        unsafe {
            redefine_as_go_string!(subspace);
            redefine_as_go_string!(type_url);
            let pset = GetParamSet(self.id, subspace, type_url);
            let pset = RawResult::from_non_null_ptr(pset).into_result()?;
            let pset = P::decode(pset.as_slice()).map_err(DecodeError::ProtoDecodeError)?;
            Ok(pset)
        }
    }
}

impl<'a> Runner<'a> for BaseApp {
    fn execute_multiple<M, R>(
        &self,
        msgs: &[(M, &str)],
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<R>
    where
        M: ::prost::Message,
        R: ::prost::Message + Default,
    {
        let msgs = msgs
            .iter()
            .map(|(msg, type_url)| {
                let mut buf = Vec::new();
                M::encode(msg, &mut buf).map_err(EncodeError::ProtoEncodeError)?;

                Ok(cosmrs::Any {
                    type_url: type_url.to_string(),
                    value: buf,
                })
            })
            .collect::<Result<Vec<cosmrs::Any>, RunnerError>>()?;

        self.execute_multiple_raw(msgs, signer)
    }

    fn execute_multiple_raw<R>(
        &self,
        msgs: Vec<cosmrs::Any>,
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<R>
    where
        R: ::prost::Message + Default,
    {
        unsafe {
            let fee = match &signer.fee_setting() {
                FeeSetting::Auto { .. } => self.estimate_fee(msgs.clone(), signer)?,
                FeeSetting::Custom { amount, gas_limit } => Fee::from_amount_and_gas(
                    cosmrs::Coin {
                        denom: amount.denom.parse().unwrap(),
                        amount: amount.amount.to_string().parse().unwrap(),
                    },
                    *gas_limit,
                ),
            };

            let tx = self.create_signed_tx(msgs.clone(), signer, fee)?;
            let base64_tx_bytes = BASE64_STANDARD.encode(tx);

            redefine_as_go_string!(base64_tx_bytes);

            let res = FinalizeBlock(self.id, base64_tx_bytes);
            let res = RawResult::from_non_null_ptr(res).into_result()?;

            let res = ResponseFinalizeBlock::decode(res.as_slice())
                .unwrap()
                .try_into();

            res
        }
    }

    fn query<Q, R>(&self, path: &str, q: &Q) -> RunnerResult<R>
    where
        Q: ::prost::Message,
        R: ::prost::Message + Default,
    {
        let mut buf = Vec::new();

        Q::encode(q, &mut buf).map_err(EncodeError::ProtoEncodeError)?;

        let base64_query_msg_bytes = BASE64_STANDARD.encode(buf);

        redefine_as_go_string!(path);
        redefine_as_go_string!(base64_query_msg_bytes);

        unsafe {
            let res = Query(self.id, path, base64_query_msg_bytes);
            let res = RawResult::from_non_null_ptr(res).into_result()?;
            R::decode(res.as_slice())
                .map_err(DecodeError::ProtoDecodeError)
                .map_err(RunnerError::DecodeError)
        }
    }
}
