-- Credit cards: track the credit line. Available = credit_limit + balance
-- (a card balance is negative when you owe). Used = -balance.
-- Savings accounts: TREA (annual yield) in basis points (550 = 5.50%).

alter table accounts add column if not exists credit_limit_cents bigint;
alter table accounts add column if not exists trea_bps integer;
