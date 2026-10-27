PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    login INTEGER NOT NULL,
    server TEXT NOT NULL,
    display_name TEXT NOT NULL,
    company TEXT NOT NULL DEFAULT '',
    currency TEXT NOT NULL DEFAULT 'USD',
    leverage INTEGER,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'active', 'archived')),
    broker_utc_offset_seconds INTEGER NOT NULL DEFAULT 0,
    balance REAL NOT NULL DEFAULT 0,
    equity REAL NOT NULL DEFAULT 0,
    last_heartbeat_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (login, server)
);

CREATE TABLE IF NOT EXISTS account_snapshots (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    taken_at TEXT NOT NULL,
    balance REAL NOT NULL DEFAULT 0,
    equity REAL NOT NULL DEFAULT 0,
    margin REAL NOT NULL DEFAULT 0,
    free_margin REAL NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS strategies (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    color TEXT NOT NULL DEFAULT '#6d7cff',
    archived INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS strategy_items (
    id TEXT PRIMARY KEY,
    strategy_id TEXT NOT NULL REFERENCES strategies(id) ON DELETE CASCADE,
    section TEXT NOT NULL DEFAULT '',
    text TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    required INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_strategy_items_strategy ON strategy_items(strategy_id, sort_order);

CREATE TABLE IF NOT EXISTS day_journals (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    date TEXT NOT NULL,
    general_html TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (account_id, date)
);

CREATE INDEX IF NOT EXISTS idx_day_journals_date ON day_journals(date);

CREATE TABLE IF NOT EXISTS day_assets (
    id TEXT PRIMARY KEY,
    journal_id TEXT NOT NULL REFERENCES day_journals(id) ON DELETE CASCADE,
    symbol TEXT NOT NULL,
    strategy_id TEXT REFERENCES strategies(id) ON DELETE SET NULL,
    journal_html TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (journal_id, symbol)
);

CREATE INDEX IF NOT EXISTS idx_day_assets_symbol ON day_assets(symbol);

CREATE TABLE IF NOT EXISTS plan_items (
    id TEXT PRIMARY KEY,
    day_asset_id TEXT NOT NULL REFERENCES day_assets(id) ON DELETE CASCADE,
    template_item_id TEXT,
    section TEXT NOT NULL DEFAULT '',
    text TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    required INTEGER NOT NULL DEFAULT 1,
    checked INTEGER NOT NULL DEFAULT 0,
    checked_at TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_plan_items_template
    ON plan_items(day_asset_id, template_item_id)
    WHERE template_item_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS symbols (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    symbol TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    digits INTEGER NOT NULL DEFAULT 5,
    tick_size REAL NOT NULL DEFAULT 0,
    tick_value REAL NOT NULL DEFAULT 0,
    contract_size REAL NOT NULL DEFAULT 1,
    currency_profit TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account_id, symbol)
);

CREATE TABLE IF NOT EXISTS trades (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    day_asset_id TEXT REFERENCES day_assets(id) ON DELETE SET NULL,
    position_ticket INTEGER NOT NULL,
    symbol TEXT NOT NULL,
    side TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
    volume_initial REAL NOT NULL,
    volume_current REAL NOT NULL,
    open_time_server TEXT NOT NULL,
    open_time_utc TEXT NOT NULL,
    close_time_server TEXT,
    close_time_utc TEXT,
    open_price REAL NOT NULL,
    close_price REAL,
    sl REAL,
    tp REAL,
    profit_gross REAL NOT NULL DEFAULT 0,
    commission REAL NOT NULL DEFAULT 0,
    swap REAL NOT NULL DEFAULT 0,
    profit_net REAL NOT NULL DEFAULT 0,
    magic INTEGER,
    comment TEXT NOT NULL DEFAULT '',
    strategy_id TEXT REFERENCES strategies(id) ON DELETE SET NULL,
    risk_amount REAL,
    r_multiple REAL,
    notes TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'closed')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (account_id, position_ticket)
);

CREATE INDEX IF NOT EXISTS idx_trades_account_close ON trades(account_id, close_time_utc);
CREATE INDEX IF NOT EXISTS idx_trades_symbol ON trades(symbol);
CREATE INDEX IF NOT EXISTS idx_trades_strategy ON trades(strategy_id);
CREATE INDEX IF NOT EXISTS idx_trades_day_asset ON trades(day_asset_id);

CREATE TABLE IF NOT EXISTS deals (
    id TEXT PRIMARY KEY,
    trade_id TEXT NOT NULL REFERENCES trades(id) ON DELETE CASCADE,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    deal_ticket INTEGER NOT NULL,
    entry INTEGER NOT NULL,
    type INTEGER NOT NULL,
    volume REAL NOT NULL,
    price REAL NOT NULL,
    profit REAL NOT NULL DEFAULT 0,
    commission REAL NOT NULL DEFAULT 0,
    swap REAL NOT NULL DEFAULT 0,
    time_server TEXT NOT NULL,
    time_utc TEXT NOT NULL,
    comment TEXT NOT NULL DEFAULT '',
    UNIQUE (account_id, deal_ticket)
);

CREATE INDEX IF NOT EXISTS idx_deals_trade ON deals(trade_id, time_server);

CREATE TABLE IF NOT EXISTS trade_series (
    id TEXT PRIMARY KEY,
    trade_id TEXT NOT NULL REFERENCES trades(id) ON DELETE CASCADE,
    timeframe TEXT NOT NULL,
    from_time INTEGER NOT NULL,
    to_time INTEGER NOT NULL,
    bar_count INTEGER NOT NULL DEFAULT 0,
    bars_json TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    UNIQUE (trade_id, timeframe)
);

CREATE TABLE IF NOT EXISTS images (
    id TEXT PRIMARY KEY,
    day_journal_id TEXT REFERENCES day_journals(id) ON DELETE CASCADE,
    day_asset_id TEXT REFERENCES day_assets(id) ON DELETE CASCADE,
    trade_id TEXT REFERENCES trades(id) ON DELETE CASCADE,
    file_name TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    thumb_path TEXT,
    mime TEXT NOT NULL,
    width INTEGER,
    height INTEGER,
    bytes INTEGER NOT NULL,
    caption TEXT NOT NULL DEFAULT '',
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    CHECK (
        (
            (CASE WHEN day_journal_id IS NULL THEN 0 ELSE 1 END) +
            (CASE WHEN day_asset_id IS NULL THEN 0 ELSE 1 END) +
            (CASE WHEN trade_id IS NULL THEN 0 ELSE 1 END)
        ) = 1
    )
);

CREATE TABLE IF NOT EXISTS tags (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    color TEXT NOT NULL DEFAULT '#8b93a7',
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS trade_tags (
    trade_id TEXT NOT NULL REFERENCES trades(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (trade_id, tag_id)
);

CREATE TABLE IF NOT EXISTS mt5_commands (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sent', 'done', 'failed')),
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mt5_commands_account_status ON mt5_commands(account_id, status, created_at);

CREATE TABLE IF NOT EXISTS mt5_event_log (
    id TEXT PRIMARY KEY,
    account_id TEXT,
    event_type TEXT NOT NULL,
    status TEXT NOT NULL,
    payload_json TEXT,
    error TEXT,
    received_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mt5_event_log_received ON mt5_event_log(received_at);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
