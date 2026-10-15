-- Checksum verification of downloads (see src/engine/verify.rs). Packages being verified use the
-- stage 'verify'.

-- Digest reported by the provider or plugin that resolved the link: "<algo>:<hex>".
ALTER TABLE downloads ADD COLUMN expected_hash TEXT;
-- Result of the last verification: algorithm (crc32, md5, sha1, sha256, sha512) and status
-- (ok, mismatch); NULL when the file was not verified.
ALTER TABLE downloads ADD COLUMN checksum_algo TEXT;
ALTER TABLE downloads ADD COLUMN checksum_status TEXT;
-- Fresh downloads triggered by a mismatch.
ALTER TABLE downloads ADD COLUMN checksum_retries INTEGER NOT NULL DEFAULT 0;
