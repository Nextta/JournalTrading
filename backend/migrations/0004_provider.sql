ALTER TABLE accounts ADD COLUMN provider TEXT NOT NULL DEFAULT 'mt5';

CREATE INDEX IF NOT EXISTS idx_accounts_provider ON accounts(provider);
