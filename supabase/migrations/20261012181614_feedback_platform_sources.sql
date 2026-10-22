-- Add provider labels only. No changes to private access, retention or quotas.
begin;
alter table public.feedback_reports drop constraint feedback_reports_source_check;
alter table public.feedback_reports add constraint feedback_reports_source_check
  check (source in ('hitomi','danbooru','chzzk','youtube','soop','pinterest','pixiv'));

create or replace function public.feedback_v1_submit(p_id uuid, p_source text, p_comment text, p_payload jsonb)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare
  caller uuid := auth.uid();
  old_owner uuid;
begin
  if caller is null then raise exception 'Authentication required' using errcode='42501'; end if;
  if p_id is null or p_source is null or p_source not in ('hitomi','danbooru','chzzk','youtube','soop','pinterest','pixiv')
    or p_comment is null or char_length(p_comment) > 1000
    or p_payload is null or jsonb_typeof(p_payload) <> 'object'
    or octet_length(p_payload::text) > 1310720
    or p_payload->>'schemaVersion' is distinct from '1'
    or p_payload->>'source' is distinct from p_source
    or jsonb_typeof(p_payload->'events') is distinct from 'array'
    or exists (select 1 from jsonb_object_keys(p_payload) k where k not in
      ('schemaVersion','appVersion','source','createdAt','windowMinutes','eventCount','truncated','events')) then
    raise exception 'Invalid report' using errcode='22023';
  end if;
  if jsonb_array_length(p_payload->'events') > 10000 then raise exception 'Report too large' using errcode='22023'; end if;
  perform pg_advisory_xact_lock(178271, 1007);
  select owner_id into old_owner from public.feedback_reports where id = p_id;
  if found then
    if old_owner <> caller then raise exception 'Invalid receipt' using errcode='42501'; end if;
    return jsonb_build_object('receiptId', p_id);
  end if;
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
