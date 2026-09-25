-- Marca, puesta a mano desde /audit, de los pagos que se pueden dividir
-- (cuenta compartida, algo que otros te van a devolver). Solo es una marca:
-- no cambia montos ni saldos.

alter table transactions add column if not exists splittable boolean not null default false;
