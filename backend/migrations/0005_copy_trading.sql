-- Copy trading: grupos (principal + secundarias), señales de la principal,
-- órdenes enviadas a cada secundaria y estado de lo copiado.

CREATE TABLE IF NOT EXISTS copy_groups (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    master_account_id TEXT NOT NULL UNIQUE REFERENCES accounts(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS copy_followers (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES copy_groups(id) ON DELETE CASCADE,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL DEFAULT 1,
    settings_json TEXT NOT NULL DEFAULT '{}',
    paused_reason TEXT,
    paused_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (group_id, account_id)
);

CREATE INDEX IF NOT EXISTS idx_copy_followers_account ON copy_followers(account_id);

CREATE TABLE IF NOT EXISTS copy_symbol_map (
    id TEXT PRIMARY KEY,
    follower_id TEXT NOT NULL REFERENCES copy_followers(id) ON DELETE CASCADE,
    master_symbol TEXT NOT NULL,
    follower_symbol TEXT NOT NULL,
    volume_ratio REAL,
    UNIQUE (follower_id, master_symbol)
);

CREATE TABLE IF NOT EXISTS copy_signals (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES copy_groups(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    master_ref TEXT NOT NULL,
    symbol TEXT NOT NULL,
    side TEXT,
    volume REAL NOT NULL DEFAULT 0,
    price REAL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_copy_signals_group ON copy_signals(group_id, seq);

CREATE TABLE IF NOT EXISTS copy_orders (
    id TEXT PRIMARY KEY,
    signal_id TEXT REFERENCES copy_signals(id) ON DELETE CASCADE,
    group_id TEXT NOT NULL REFERENCES copy_groups(id) ON DELETE CASCADE,
    follower_id TEXT NOT NULL REFERENCES copy_followers(id) ON DELETE CASCADE,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    master_ref TEXT NOT NULL,
    tag TEXT NOT NULL,
    action TEXT NOT NULL,
    symbol TEXT NOT NULL DEFAULT '',
    side TEXT,
    volume REAL NOT NULL DEFAULT 0,
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'sent', 'filled', 'placed', 'done', 'rejected', 'skipped')),
    error TEXT,
    fill_price REAL,
    fill_volume REAL,
    latency_ms INTEGER,
    created_at TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    sent_at TEXT,
    done_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_copy_orders_account_status ON copy_orders(account_id, status, created_ms);
CREATE INDEX IF NOT EXISTS idx_copy_orders_created ON copy_orders(created_ms);

CREATE TABLE IF NOT EXISTS copy_positions (
    follower_id TEXT NOT NULL REFERENCES copy_followers(id) ON DELETE CASCADE,
    master_ref TEXT NOT NULL,
    tag TEXT NOT NULL,
    symbol TEXT NOT NULL,
    side TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
    volume REAL NOT NULL DEFAULT 0,
    follower_ref TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (follower_id, master_ref)
);

CREATE TABLE IF NOT EXISTS copy_pendings (
    follower_id TEXT NOT NULL REFERENCES copy_followers(id) ON DELETE CASCADE,
    master_order_ref TEXT NOT NULL,
    tag TEXT NOT NULL,
    symbol TEXT NOT NULL,
    follower_order_ref TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (follower_id, master_order_ref)
);
