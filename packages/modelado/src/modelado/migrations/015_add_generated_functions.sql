-- Migration: Add ikam_generated_functions table for Phase 9.4
-- Purpose: Store canonicalized generated functions with CAS deduplication
-- Guarantees: Content-addressable storage, provenance preservation, storage monotonicity

-- Generated functions table with CAS properties
CREATE TABLE IF NOT EXISTS ikam_generated_functions (
    -- Primary identifiers
    function_id TEXT PRIMARY KEY,                    -- Short ID (gfn_<hash_prefix>)
    content_hash TEXT NOT NULL UNIQUE,               -- BLAKE3/SHA256 of canonical code (CAS key)
    
    -- Code storage
    canonical_code TEXT NOT NULL,                    -- Canonicalized function code
    original_code TEXT NOT NULL,                     -- Original generated code (for debugging)
    transformations_applied JSONB DEFAULT '[]',      -- List of canonicalization steps
    is_semantically_equivalent BOOLEAN DEFAULT TRUE, -- Validation flag
    
    -- Generation metadata (provenance)
    semantic_intent TEXT,                            -- Natural language intent
    generation_strategy TEXT,                        -- Strategy used (e.g., "llm_generation", "template")
    prompt_hash TEXT,                                -- Hash of generation prompt (for reproducibility)
    model_name TEXT,                                 -- LLM model if applicable
    temperature REAL,                                -- Generation temperature
    parameters JSONB DEFAULT '{}',                   -- Additional generation params
    
    -- Storage metadata
    stored_at TIMESTAMP NOT NULL DEFAULT NOW(),      -- First storage timestamp
    storage_key TEXT NOT NULL,                       -- Backend storage key
    deduplicated BOOLEAN DEFAULT FALSE,              -- True if matched existing hash
    cache_key TEXT,                                  -- Semantic cache key (intent + params hash)
    execution_count INTEGER DEFAULT 1,               -- Number of times used
    
    -- Timestamps
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Indices for performance

-- Primary CAS lookup (most common query)
CREATE INDEX IF NOT EXISTS idx_generated_functions_content_hash 
    ON ikam_generated_functions(content_hash);

-- Semantic cache lookup (intent-based retrieval)
CREATE INDEX IF NOT EXISTS idx_generated_functions_cache_key 
    ON ikam_generated_functions(cache_key) 
    WHERE cache_key IS NOT NULL;

-- Temporal queries (recent generations)
CREATE INDEX IF NOT EXISTS idx_generated_functions_stored_at 
    ON ikam_generated_functions(stored_at DESC);

-- Deduplication analysis (filter duplicates)
CREATE INDEX IF NOT EXISTS idx_generated_functions_deduplicated 
    ON ikam_generated_functions(deduplicated) 
    WHERE deduplicated = TRUE;

-- Provenance tracking (find by intent)
CREATE INDEX IF NOT EXISTS idx_generated_functions_intent 
    ON ikam_generated_functions(semantic_intent);

-- Storage statistics view
CREATE OR REPLACE VIEW vw_generated_function_stats AS
SELECT
    COUNT(*) AS total_functions_generated,
    COUNT(DISTINCT content_hash) AS unique_functions_stored,
    COUNT(*) - COUNT(DISTINCT content_hash) AS duplicate_count,
    SUM(LENGTH(original_code)) AS raw_storage_bytes,
    SUM(DISTINCT LENGTH(canonical_code)) AS deduplicated_storage_bytes,
    SUM(LENGTH(original_code)) - SUM(DISTINCT LENGTH(canonical_code)) AS storage_savings_bytes,
    ROUND(
        (SUM(LENGTH(original_code)) - SUM(DISTINCT LENGTH(canonical_code))) * 100.0 / 
        NULLIF(SUM(LENGTH(original_code)), 0),
        2
    ) AS storage_savings_percent,
    SUM(LENGTH(original_code)) - SUM(DISTINCT LENGTH(canonical_code)) AS monotonicity_delta
FROM ikam_generated_functions;

-- Function to update updated_at timestamp
CREATE OR REPLACE FUNCTION update_generated_function_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger to auto-update updated_at
CREATE TRIGGER trigger_update_generated_function_timestamp
    BEFORE UPDATE ON ikam_generated_functions
    FOR EACH ROW
    EXECUTE FUNCTION update_generated_function_updated_at();

-- Comments for documentation
COMMENT ON TABLE ikam_generated_functions IS 
    'Phase 9.4: Canonicalized generated functions with CAS deduplication. '
    'Guarantees: (1) Idempotent storage via content_hash UNIQUE constraint, '
    '(2) Storage monotonicity Δ(N) ≥ 0, (3) Provenance preservation for Fisher Information.';

COMMENT ON COLUMN ikam_generated_functions.content_hash IS 
    'BLAKE3 or SHA256 hash of canonical_code. Enforces CAS property: unique hash = unique function.';

COMMENT ON COLUMN ikam_generated_functions.canonical_code IS 
    'Normalized code after canonicalization transformations. Used for CAS deduplication.';

COMMENT ON COLUMN ikam_generated_functions.transformations_applied IS 
    'JSON array of canonicalization steps (e.g., ["imports_sorted", "variables_renamed"]).';

COMMENT ON COLUMN ikam_generated_functions.cache_key IS 
    'Semantic cache key: hash(intent + params). Enables fast retrieval for repeated intents.';

COMMENT ON COLUMN ikam_generated_functions.execution_count IS 
    'Number of times this function was generated/retrieved. Increments on deduplication hits.';

COMMENT ON VIEW vw_generated_function_stats IS 
    'Real-time storage statistics for deduplication analysis. Validates monotonicity: Δ(N) ≥ 0.';
