use cosmwasm_std::Coin;
use injective_std::types::cosmwasm::wasm::v1::{
    AccessConfig, MsgExecuteContract, MsgExecuteContractResponse, MsgInstantiateContract,
    MsgInstantiateContractResponse, MsgMigrateContract, MsgMigrateContractResponse, MsgStoreCode,
    MsgStoreCodeResponse, QuerySmartContractStateRequest, QuerySmartContractStateResponse,
};
use serde::{de::DeserializeOwned, Serialize};

use test_tube_inj::runner::error::{DecodeError, EncodeError, RunnerError};
use test_tube_inj::runner::result::{RunnerExecuteResult, RunnerResult};
use test_tube_inj::{
    account::{Account, SigningAccount},
    runner::Runner,
};

pub struct Wasm<'a, R: Runner<'a>> {
    runner: &'a R,
}

impl<'a, R: Runner<'a>> super::Module<'a, R> for Wasm<'a, R> {
    fn new(runner: &'a R) -> Self {
        Wasm { runner }
    }
}

impl<'a, R> Wasm<'a, R>
where
    R: Runner<'a>,
{
    pub fn store_code(
        &self,
        wasm_byte_code: &[u8],
        instantiate_permission: Option<AccessConfig>,
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<MsgStoreCodeResponse> {
        self.runner.execute(
            MsgStoreCode {
                sender: signer.address(),
                wasm_byte_code: wasm_byte_code.to_vec(),
                instantiate_permission,
            },
            "/cosmwasm.wasm.v1.MsgStoreCode",
            signer,
        )
    }

    pub fn instantiate<M>(
        &self,
        code_id: u64,
        msg: &M,
        admin: Option<&str>,
        label: Option<&str>,
        funds: &[Coin],
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<MsgInstantiateContractResponse>
    where
        M: ?Sized + Serialize,
    {
        self.runner.execute(
            MsgInstantiateContract {
                sender: signer.address(),
                admin: admin.unwrap_or_default().to_string(),
                code_id,
                label: label.unwrap_or(" ").to_string(), // empty string causes panic
                msg: serde_json::to_vec(msg).map_err(EncodeError::JsonEncodeError)?,
                funds: funds
                    .iter()
                    .map(|c| injective_std::types::cosmos::base::v1beta1::Coin {
                        denom: c.denom.parse().unwrap(),
                        amount: format!("{}", c.amount.u128()),
                    })
                    .collect(),
            },
            "/cosmwasm.wasm.v1.MsgInstantiateContract",
            signer,
        )
    }

    pub fn execute<M>(
        &self,
        contract: &str,
        msg: &M,
        funds: &[Coin],
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<MsgExecuteContractResponse>
    where
        M: ?Sized + Serialize,
    {
        self.runner.execute(
            MsgExecuteContract {
                sender: signer.address(),
                msg: serde_json::to_vec(msg).map_err(EncodeError::JsonEncodeError)?,
                funds: funds
                    .iter()
                    .map(|c| injective_std::types::cosmos::base::v1beta1::Coin {
                        denom: c.denom.parse().unwrap(),
                        amount: format!("{}", c.amount.u128()),
                    })
                    .collect(),
                contract: contract.to_owned(),
            },
            "/cosmwasm.wasm.v1.MsgExecuteContract",
            signer,
        )
    }

    pub fn migrate<M>(
        &self,
        code_id: u64,
        contract: &str,
        msg: &M,
        signer: &SigningAccount,
    ) -> RunnerExecuteResult<MsgMigrateContractResponse>
    where
        M: ?Sized + Serialize,
    {
        self.runner.execute(
            MsgMigrateContract {
                sender: signer.address(),
                contract: contract.to_owned(),
                code_id,
                msg: serde_json::to_vec(msg).map_err(EncodeError::JsonEncodeError)?,
            },
            "/cosmwasm.wasm.v1.MsgMigrateContract",
            signer,
        )
    }

    pub fn query<M, Res>(&self, contract: &str, msg: &M) -> RunnerResult<Res>
    where
        M: ?Sized + Serialize,
        Res: ?Sized + DeserializeOwned,
    {
        let res = self
            .runner
            .query::<QuerySmartContractStateRequest, QuerySmartContractStateResponse>(
                "/cosmwasm.wasm.v1.Query/SmartContractState",
                &QuerySmartContractStateRequest {
                    address: contract.to_owned(),
                    query_data: serde_json::to_vec(msg).map_err(EncodeError::JsonEncodeError)?,
                },
            )?;

        serde_json::from_slice(&res.data)
            .map_err(DecodeError::JsonDecodeError)
            .map_err(RunnerError::DecodeError)
    }
}
