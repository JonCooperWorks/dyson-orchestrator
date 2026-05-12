ALTER TABLE proxy_tokens ADD COLUMN token_lookup TEXT;

CREATE INDEX idx_proxy_tokens_lookup
  ON proxy_tokens(token_lookup);
