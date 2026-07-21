-- The real transaction sources go beyond the original enum (interbank,
-- scotiabank, ...). Drop the CHECK so `kind` is free text.

alter table accounts drop constraint if exists accounts_kind_check;
