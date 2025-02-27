use casper_dao_utils::TestContract;

use crate::common::{
    params::{Account, TokenId},
    DaoWorld,
};

#[allow(dead_code)]
impl DaoWorld {
    pub fn checked_mint_kyc_token(
        &mut self,
        minter: &Account,
        recipient: &Account,
    ) -> Result<(), casper_dao_utils::Error> {
        let minter = self.get_address(minter);
        let recipient = self.get_address(recipient);

        self.kyc_token.as_account(minter).mint(recipient)
    }

    pub fn mint_kyc_token(&mut self, minter: &Account, recipient: &Account) {
        self.checked_mint_kyc_token(minter, recipient)
            .expect("A KYC Token should be minted successfully");
    }

    pub fn checked_burn_kyc_token(
        &mut self,
        burner: &Account,
        holder: &Account,
    ) -> Result<(), casper_dao_utils::Error> {
        let burner = self.get_address(burner);
        let holder = self.get_address(holder);

        self.kyc_token.as_account(burner).burn(holder)
    }

    pub fn burn_kyc_token(&mut self, minter: &Account, recipient: &Account) {
        self.checked_burn_kyc_token(minter, recipient)
            .expect("A token should be burned");
    }

    pub fn get_kyc_token_id(&self, holder: &Account) -> TokenId {
        let holder = self.get_address(holder);
        let id = self
            .kyc_token
            .token_id(holder)
            .expect("Holder should own a token");
        TokenId(id)
    }

    pub fn is_account_kyced(&self, account: &Account) -> bool {
        let address = self.get_address(account);

        !self.kyc_token.balance_of(address).is_zero()
    }
}
