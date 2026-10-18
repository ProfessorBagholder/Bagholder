-- The sightings that state a venue and those that state none, each indexed by
-- instrument and place (brief 44, 2): a change to an instrument's names re-forms
-- them only up to the next sighting that states a venue, found by one seek on
-- these, never by reading every sighting after the change.
CREATE INDEX instrument_sightings_stating ON instrument_sightings (instrument_id, day, symbol) WHERE venue_mic IS NOT NULL OR venue_name IS NOT NULL;
CREATE INDEX instrument_sightings_silent ON instrument_sightings (instrument_id, day, symbol) WHERE venue_mic IS NULL AND venue_name IS NULL;
