-- Statistics count each file and each package once (see src/stats.rs): a file downloaded again
-- after a checksum mismatch or a retry, or a package extracted again from the history, is not
-- counted a second time.

ALTER TABLE downloads ADD COLUMN counted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE packages ADD COLUMN counted INTEGER NOT NULL DEFAULT 0;

-- What the statistics already include (the backfill of 0005, then every completion).
UPDATE downloads SET counted = 1 WHERE (status = 'completed' AND finished_at IS NOT NULL) OR checksum_retries > 0;
UPDATE packages SET counted = 1 WHERE finished_at IS NOT NULL AND error IS NULL;
