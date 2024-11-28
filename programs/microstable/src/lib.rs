use anchor_lang::prelude::*;
use anchor_spl::{
    token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked, CloseAccount, transfer_checked, close_account},
    associated_token::AssociatedToken,
};

mod instructions;
pub use instructions::*;
declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

#[program]
pub mod manager {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, min_collat_ratio: u64) -> Result<()> {
        let state = &mut ctx.accounts.state;        
        // Initialize state
        state.min_collat_ratio = min_collat_ratio;
        state.weth_mint = ctx.accounts.weth_mint.key();
        state.shusd_mint = ctx.accounts.shusd_mint.key();
        state.authority = ctx.accounts.deployer.key();
        state.bump = ctx.bumps.state;

        // Validation
        require!(
            min_collat_ratio >= 150, // 100% minimum
            ErrorCode::InvalidCollateralRatio
        );

        Ok(())
    }

    // the function that deposits the weth into the vault and than mints corresponding shusd
    pub fn deposit_weth_mint_shusd(ctx: Context<DepositWeth>) -> Result<()> {
        let amount = ctx.accounts.deposit_state.amount;
        // require amount is greater than 0
        require!(amount > 0, ErrorCode::InvalidAmount);
        deposit_weth(ctx, amount)?;
        Ok(())
    }

}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub deployer: Signer<'info>,

    #[account(
        init,
        payer = deployer,
        space = 8 + State::INIT_SPACE,  
        seeds = [b"state".as_ref()],
        bump
    )]
    pub state: Account<'info, State>,

    /// The WETH mint address
    pub weth_mint: InterfaceAccount<'info, Mint>,
    
    /// The shUSD mint address
    pub shusd_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,

    pub system_program: Program<'info, System>,
}

// so the functionality we want are deposit, withdraw and liquidate - within deposit we do minting, within withdraw we do burning and within liquidate we do a burning too
// but in liquidate the signer is not the user that hold the shusd tokens, so we will use the freeze authority to do the burning and than give the underlying weth to the user
// another thing is we will need a vault where we will be storing all the weth and the authority of the vault will be this contract, well not this contract but some other account 

#[derive(Accounts)]
pub struct DepositWeth<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,

    #[account(
        init,
        payer = depositor,
        associated_token::mint = weth_mint,
        associated_token::authority = deposit_state,
        associated_token::token_program = token_program,
    )]
    pub vault_weth: InterfaceAccount<'info, TokenAccount>,

    // because we are initializing here, we will need to save inner
    #[account(
        init,
        payer = depositor,
        space = DepositState::INIT_SPACE,
        seeds = [b"deposit_state".as_ref(), depositor.key().as_ref()],
        bump
    )]
    pub deposit_state: Account<'info, DepositState>,

    pub weth_mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,

    #[account(
        mut,
        associated_token::mint = weth_mint,
        associated_token::authority = depositor,
        associated_token::token_program = token_program,
    )]
    pub depositor_weth_account: InterfaceAccount<'info, TokenAccount>,


    pub system_program: Program<'info, System>,
    // only one associated token program for all the accounts
    pub associated_token_program: Program<'info, AssociatedToken>,
}

// withdraw
#[derive(Accounts)]
pub struct WithdrawWeth<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,

    #[account(
        mut,
        associated_token::mint = weth_mint,
        associated_token::authority = depositor,
        associated_token::token_program = token_program,
    )]
    pub depositor_weth_account: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = weth_mint,
        associated_token::authority = deposit_state,
        associated_token::token_program = token_program,
    )]
    pub vault_weth: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        close = depositor,
        seeds = [b"deposit_state".as_ref(), depositor.key().as_ref()],
        bump = deposit_state.bump,
    )]
    pub deposit_state: Account<'info, DepositState>,        


    pub weth_mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,

    pub system_program: Program<'info, System>,
}

#[account]
#[derive(InitSpace)]
pub struct DepositState {
    pub amount: u64,
    pub bump: u8,
}

#[account]
// init space
#[derive(InitSpace)]
pub struct State {
    pub min_collat_ratio: u64,
    pub weth_mint: Pubkey,
    pub shusd_mint: Pubkey,
    pub authority: Pubkey,  // The admin who can update parameters
    pub bump: u8,
}

fn deposit_weth(ctx: Context<DepositWeth>, amount: u64) -> Result<()> {
    // we will transfer tokens from the signer to the vault
    transfer_tokens(
        &ctx.accounts.depositor_weth_account,
        &ctx.accounts.vault_weth,
        &amount,
        &ctx.accounts.weth_mint,
        &ctx.accounts.depositor,
        &ctx.accounts.token_program,
    )?;

    // set the deposit state
    ctx.accounts.deposit_state.set_inner(DepositState {
        amount,
        bump: ctx.bumps.deposit_state,
    });

    Ok(())
}

fn withdraw_weth(ctx: Context<WithdrawWeth>) -> Result<()> {
    let seeds = [b"deposit_state".as_ref(), ctx.accounts.depositor.key().as_ref(), &[ctx.bumps.deposit_state]];
    let signer = &[&seeds[..]];

    let accounts = TransferChecked{
        from: ctx.accounts.vault_weth.to_account_info(),
        mint: ctx.accounts.weth_mint.to_account_info(),
        to: ctx.accounts.depositor_weth_account.to_account_info(),
        authority: ctx.accounts.deposit_state.to_account_info(),
    };

    let cpi_context = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), accounts, signer);



    transfer_checked(cpi_context, ctx.accounts.deposit_state.amount, ctx.accounts.weth_mint.decimals)?;

    let close_accounts = CloseAccount{
        account: ctx.accounts.vault_weth.to_account_info(),
        destination: ctx.accounts.depositor.to_account_info(),
        authority: ctx.accounts.deposit_state.to_account_info(),
    };

    let cpi_context = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), close_accounts, signer);
    close_account(cpi_context)?;

    Ok(())
}
  
#[error_code]
pub enum ErrorCode {
    #[msg("Collateral ratio below minimum")]
    CollateralRatioTooLow,
    #[msg("Invalid collateral ratio provided")]
    InvalidCollateralRatio,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Protocol has already been initialized")]
    AlreadyInitialized,
    #[msg("Invalid amount")]
    InvalidAmount,
}