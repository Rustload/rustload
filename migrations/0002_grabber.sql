-- Link grabber: links are looked up by URL to flag duplicates before they are added.
CREATE INDEX downloads_url ON downloads(url);
