CREATE TABLE IF NOT EXISTS asset_groups (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    color TEXT NOT NULL DEFAULT '#6d7cff',
    default_strategy_id TEXT REFERENCES strategies(id) ON DELETE SET NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS asset_group_items (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES asset_groups(id) ON DELETE CASCADE,
    symbol TEXT NOT NULL,
    strategy_id TEXT REFERENCES strategies(id) ON DELETE SET NULL,
    strategy_overridden INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    UNIQUE (group_id, symbol)
);

CREATE INDEX IF NOT EXISTS idx_asset_group_items_group
    ON asset_group_items(group_id, sort_order);
