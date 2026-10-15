-- Media sites downloaded through yt-dlp.

-- JSON MediaOptions (quality, audio format, subtitles...); NULL when the package has no media link.
ALTER TABLE packages ADD COLUMN media TEXT;

-- JSON MediaMeta (title, duration, thumbnail...) of a media link, filled when it is probed.
ALTER TABLE downloads ADD COLUMN media TEXT;
