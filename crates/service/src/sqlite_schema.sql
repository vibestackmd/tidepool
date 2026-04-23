-- Tidepool unified SQLite schema. Consumed by `SqliteBackend::open`.
-- Tables are prefix-namespaced by store (`cnft_*`, `cache_*`,
-- `webhooks_*`) so `sqlite3 ./tidepool.sqlite '.tables'` reads
-- coherently.

-- ─── cNFT indexer state ─────────────────────────────────────────
CREATE TABLE IF NOT EXISTS cnft_trees (
    tree_pubkey BLOB PRIMARY KEY,
    depth INTEGER NOT NULL,
    max_buffer_size INTEGER NOT NULL,
    num_minted INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS cnft_leaves (
    rowid_alias INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id BLOB NOT NULL UNIQUE,
    tree BLOB NOT NULL,
    nonce INTEGER NOT NULL,
    leaf_index INTEGER NOT NULL,
    owner BLOB NOT NULL,
    delegate BLOB NOT NULL,
    data_hash BLOB NOT NULL,
    creator_hash BLOB NOT NULL,
    leaf_hash BLOB NOT NULL,
    burned INTEGER NOT NULL,
    mint_metadata_json BLOB NOT NULL,
    UNIQUE(tree, leaf_index)
);
CREATE INDEX IF NOT EXISTS idx_cnft_leaves_by_tree ON cnft_leaves(tree);

CREATE TABLE IF NOT EXISTS cnft_last_sig (
    tree BLOB PRIMARY KEY,
    signature TEXT NOT NULL
);

-- ─── DAS cache (assets + secondary indexes) ────────────────────
CREATE TABLE IF NOT EXISTS cache_assets (
    id TEXT PRIMARY KEY,
    json BLOB NOT NULL,
    interface TEXT NOT NULL,
    burnt INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS cache_by_owner (
    owner TEXT NOT NULL,
    asset_id TEXT NOT NULL,
    PRIMARY KEY (owner, asset_id)
);
CREATE INDEX IF NOT EXISTS idx_cache_by_owner_asset ON cache_by_owner(asset_id);

CREATE TABLE IF NOT EXISTS cache_by_authority (
    authority TEXT NOT NULL,
    asset_id TEXT NOT NULL,
    PRIMARY KEY (authority, asset_id)
);
CREATE INDEX IF NOT EXISTS idx_cache_by_authority_asset ON cache_by_authority(asset_id);

CREATE TABLE IF NOT EXISTS cache_by_creator (
    creator TEXT NOT NULL,
    asset_id TEXT NOT NULL,
    PRIMARY KEY (creator, asset_id)
);
CREATE INDEX IF NOT EXISTS idx_cache_by_creator_asset ON cache_by_creator(asset_id);

CREATE TABLE IF NOT EXISTS cache_by_group (
    group_key TEXT NOT NULL,
    group_value TEXT NOT NULL,
    asset_id TEXT NOT NULL,
    PRIMARY KEY (group_key, group_value, asset_id)
);
CREATE INDEX IF NOT EXISTS idx_cache_by_group_asset ON cache_by_group(asset_id);

CREATE TABLE IF NOT EXISTS cache_master_editions (
    master_mint TEXT PRIMARY KEY,
    master_edition_pda TEXT NOT NULL,
    supply INTEGER NOT NULL,
    max_supply INTEGER
);

CREATE TABLE IF NOT EXISTS cache_print_editions (
    print_mint TEXT PRIMARY KEY,
    print_edition_pda TEXT NOT NULL,
    parent_master_edition_pda TEXT NOT NULL,
    edition_num INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_cache_prints_by_parent ON cache_print_editions(parent_master_edition_pda);

-- ─── Webhook registry ───────────────────────────────────────────
CREATE TABLE IF NOT EXISTS webhooks_webhooks (
    webhook_id TEXT PRIMARY KEY,
    json BLOB NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
