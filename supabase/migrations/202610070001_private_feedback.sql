-- Explicit, private diagnostic reports. No public read access or Storage bucket.
begin;
create table public.feedback_reports (
  id uuid primary key,
  owner_id uuid not null,
  source text not null check (source in ('hitomi','danbooru','chzzk')),
  comment text not null check (char_length(comment) <= 1000),
  payload jsonb not null check (jsonb_typeof(payload) = 'object' and octet_length(payload::text) <= 262144),
  created_at timestamptz not null default now()
);
create index feedback_reports_owner_created on public.feedback_reports(owner_id, created_at);
alter table public.feedback_reports enable row level security;
revoke all on table public.feedback_reports from public, anon, authenticated;
grant all on table public.feedback_reports to service_role;

create function public.feedback_v1_submit(p_id uuid, p_source text, p_comment text, p_payload jsonb)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
  caller uuid := auth.uid();
  old_owner uuid;
begin
  if caller is null then raise exception 'Authentication required' using errcode='42501'; end if;
  if p_id is null or p_source is null or p_source not in ('hitomi','danbooru','chzzk')
    or p_comment is null or char_length(p_comment) > 1000
    or p_payload is null or jsonb_typeof(p_payload) <> 'object'
    or octet_length(p_payload::text) > 262144
    or p_payload->>'schemaVersion' is distinct from '1'
    or p_payload->>'source' is distinct from p_source
    or jsonb_typeof(p_payload->'events') is distinct from 'array'
    or exists (select 1 from jsonb_object_keys(p_payload) k where k not in
      ('schemaVersion','appVersion','source','createdAt','windowMinutes','eventCount','truncated','events')) then
    raise exception 'Invalid report' using errcode='22023';
  end if;
  if jsonb_array_length(p_payload->'events') > 2000 then raise exception 'Report too large' using errcode='22023'; end if;
  -- One bounded queue, including simultaneous submissions and anonymous-account abuse.
  perform pg_advisory_xact_lock(178271, 1007);
  select owner_id into old_owner from public.feedback_reports where id = p_id;
  if found then
    if old_owner <> caller then raise exception 'Invalid receipt' using errcode='42501'; end if;
    return jsonb_build_object('receiptId', p_id);
  end if;
  -- Expired reports are cleaned on new submission; not advertised as a timed purge.
  delete from public.feedback_reports where created_at < now() - interval '30 days';
  if (select count(*) from public.feedback_reports where owner_id = caller and created_at > now() - interval '1 day') >= 5
    or (select count(*) from public.feedback_reports where created_at > now() - interval '1 day') >= 50
    or (select coalesce(sum(octet_length(payload::text) + octet_length(comment)),0) from public.feedback_reports) + octet_length(p_payload::text) + octet_length(p_comment) > 67108864 then
    raise exception 'Report limit reached. Try later.' using errcode='P0001';
  end if;
  insert into public.feedback_reports(id,owner_id,source,comment,payload) values (p_id,caller,p_source,p_comment,p_payload);
  return jsonb_build_object('receiptId',p_id);
end;
$$;
revoke all on function public.feedback_v1_submit(uuid,text,text,jsonb) from public, anon;
grant execute on function public.feedback_v1_submit(uuid,text,text,jsonb) to authenticated;
commit;
