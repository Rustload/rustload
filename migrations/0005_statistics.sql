-- Download statistics. Independent of the history, so clearing it keeps the numbers.

-- Completed files, aggregated per local day and hour, host and handler.
CREATE TABLE stats_transfers (
    day      TEXT    NOT NULL,             -- local date, YYYY-MM-DD
    hour     INTEGER NOT NULL,             -- local hour of completion, 0..23
    host     TEXT    NOT NULL,             -- host of the link, '' when unknown
    via      TEXT    NOT NULL,             -- 'direct', debrid provider id or hoster plugin name
    bytes    INTEGER NOT NULL,
    files    INTEGER NOT NULL,
    secs     REAL    NOT NULL,             -- time spent transferring
    first_at INTEGER NOT NULL,             -- completion time of the first file counted in the row
    PRIMARY KEY (day, hour, host, via)
) WITHOUT ROWID;

-- Daily counters: finished packages, failed files and the highest total speed (bytes/s).
CREATE TABLE stats_days (
    day        TEXT    PRIMARY KEY,
    packages   INTEGER NOT NULL DEFAULT 0,
    failed     INTEGER NOT NULL DEFAULT 0,
    peak_speed INTEGER
) WITHOUT ROWID;

-- Backfill from what is already completed. The transfer time of these files is approximated by
-- the time between their first start and their completion.
INSERT INTO stats_transfers (day, hour, host, via, bytes, files, secs, first_at)
SELECT date(finished_at, 'unixepoch', 'localtime'),
       CAST(strftime('%H', finished_at, 'unixepoch', 'localtime') AS INTEGER),
       COALESCE(host, ''),
       COALESCE(via, 'direct'),
       SUM(MAX(COALESCE(size, downloaded), 0)),
       COUNT(*),
       SUM(MAX(finished_at - COALESCE(started_at, finished_at), 1)),
       MIN(finished_at)
FROM downloads
WHERE status = 'completed' AND finished_at IS NOT NULL
GROUP BY 1, 2, 3, 4;

INSERT INTO stats_days (day, packages)
SELECT date(finished_at, 'unixepoch', 'localtime'), COUNT(*)
FROM packages
WHERE finished_at IS NOT NULL AND error IS NULL
GROUP BY 1;
