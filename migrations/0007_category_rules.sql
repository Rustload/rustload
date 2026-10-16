-- A category can claim packages with a rule (see src/engine/storage.rs): a regular expression
-- matched against the package name, its links and its file names, before the extensions.

ALTER TABLE categories ADD COLUMN pattern TEXT NOT NULL DEFAULT '';
