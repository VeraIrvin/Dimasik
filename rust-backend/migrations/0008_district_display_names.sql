-- Version 8: retain each district's canonical historical name while allowing
-- an editorial display-name override. NULL means the canonical name is current.
ALTER TABLE districts ADD COLUMN display_name TEXT;
