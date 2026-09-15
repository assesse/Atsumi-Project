-- Applied via the Supabase SQL editor on 2026-09-15 KST.
-- Validated on PostgreSQL with supabase/tests/community_security.sql (ROLLBACK).
-- Atsumi Community v1. Separate from the desktop/library database.
-- All data lives in an unexposed schema. Only bounded RPCs are exposed.
begin;

create schema atsumi_community;
revoke all on schema atsumi_community from public, anon, authenticated;

create table atsumi_community.members (
  id uuid primary key default gen_random_uuid(),
  auth_user_id uuid not null unique references auth.users(id) on delete cascade,
  nickname text not null check (char_length(nickname) between 2 and 24),
  enabled boolean not null default true,
  last_write_at timestamptz,
  write_day date not null default current_date,
  daily_writes integer not null default 0,
  created_at timestamptz not null default now()
);

create table atsumi_community.reviews (
  id uuid primary key default gen_random_uuid(),
  member_id uuid not null references atsumi_community.members(id) on delete cascade,
  source text not null check (source in ('hitomi', 'danbooru')),
  work_id text not null check (work_id ~ '^[1-9][0-9]{0,19}$'),
  rating smallint not null check (rating between 1 and 5),
  recommended boolean not null default false,
  comment text not null default '' check (char_length(comment) <= 500),
  hidden boolean not null default false,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (member_id, source, work_id)
);
create index reviews_work_recent on atsumi_community.reviews(source, work_id, created_at desc, id desc) where not hidden;
create index reviews_feed_recent on atsumi_community.reviews(created_at desc, id desc) where not hidden;
create index reviews_source_recent on atsumi_community.reviews(source, created_at desc, id desc) where not hidden;

create table atsumi_community.work_stats (
  source text not null,
  work_id text not null,
  review_count bigint not null default 0 check (review_count >= 0),
  rating_sum bigint not null default 0 check (rating_sum >= 0),
  recommendation_count bigint not null default 0 check (recommendation_count >= 0),
  updated_at timestamptz not null default now(),
  primary key (source, work_id)
);

create table atsumi_community.reports (
  review_id uuid not null references atsumi_community.reviews(id) on delete cascade,
  member_id uuid not null references atsumi_community.members(id) on delete cascade,
  reason text not null check (char_length(reason) between 1 and 300),
  created_at timestamptz not null default now(),
  primary key (review_id, member_id)
);

alter table atsumi_community.members enable row level security;
alter table atsumi_community.reviews enable row level security;
alter table atsumi_community.work_stats enable row level security;
alter table atsumi_community.reports enable row level security;
revoke all on all tables in schema atsumi_community from public, anon, authenticated;

-- No client table grants or RLS bypass roles. Definer RPCs explicitly enforce
-- identity/ownership; a public/publishable key alone cannot write anything.
create function atsumi_community.require_member() returns uuid
language plpgsql security definer set search_path = '' as $$
declare v_member uuid;
begin
  if auth.uid() is null then
    raise exception using errcode = '28000', message = 'COMMUNITY_LOGIN_REQUIRED';
  end if;
  select id into v_member from atsumi_community.members where auth_user_id = auth.uid() and enabled;
  if v_member is null then
    raise exception using errcode = '42501', message = 'COMMUNITY_PROFILE_REQUIRED';
  end if;
  return v_member;
end;
$$;

create function atsumi_community.check_work(p_source text, p_work_id text) returns void
language plpgsql set search_path = '' as $$
begin
  if p_source is null or p_source not in ('hitomi', 'danbooru')
    or p_work_id is null or p_work_id !~ '^[1-9][0-9]{0,19}$' then
    raise exception using errcode = '22023', message = 'COMMUNITY_INVALID_WORK';
  end if;
end;
$$;

create function atsumi_community.rate_limit(p_member uuid) returns void
language plpgsql security definer set search_path = '' as $$
declare v_member atsumi_community.members;
begin
  select * into v_member from atsumi_community.members where id = p_member for update;
  if not found or not v_member.enabled then raise exception 'COMMUNITY_ACCOUNT_DISABLED'; end if;
  if v_member.last_write_at is not null and v_member.last_write_at > clock_timestamp() - interval '3 seconds' then
    raise exception using errcode = 'P0001', message = 'COMMUNITY_WRITE_TOO_FAST';
  end if;
  if v_member.write_day = current_date and v_member.daily_writes >= 100 then
    raise exception 'COMMUNITY_DAILY_LIMIT';
  end if;
  update atsumi_community.members set last_write_at = clock_timestamp(), write_day = current_date,
    daily_writes = case when write_day = current_date then daily_writes + 1 else 1 end where id = p_member;
end;
$$;

-- Incremental summaries are updated in the same transaction as the review.
-- Members are immutable on a review; all RPCs also keep its work key immutable.
create function atsumi_community.update_stats() returns trigger
language plpgsql security definer set search_path = '' as $$
declare
  v_source text; v_work text;
  v_count bigint := 0; v_rating bigint := 0; v_recommended bigint := 0;
begin
  if tg_op <> 'INSERT' then
    v_source := old.source; v_work := old.work_id;
    if not old.hidden then
      v_count := v_count - 1; v_rating := v_rating - old.rating;
      v_recommended := v_recommended - old.recommended::integer;
    end if;
  end if;
  if tg_op <> 'DELETE' then
    v_source := new.source; v_work := new.work_id;
    if tg_op = 'UPDATE' and (new.source, new.work_id, new.member_id, new.created_at)
      is distinct from (old.source, old.work_id, old.member_id, old.created_at) then
      raise exception 'COMMUNITY_IMMUTABLE_REVIEW_IDENTITY';
    end if;
    if not new.hidden then
      v_count := v_count + 1; v_rating := v_rating + new.rating;
      v_recommended := v_recommended + new.recommended::integer;
    end if;
  end if;
  -- Create the row with zero first: negative deltas must not violate an INSERT
  -- check constraint before ON CONFLICT reaches the existing summary row.
  insert into atsumi_community.work_stats(source, work_id) values (v_source, v_work)
    on conflict (source, work_id) do nothing;
  update atsumi_community.work_stats
    set review_count = review_count + v_count, rating_sum = rating_sum + v_rating,
      recommendation_count = recommendation_count + v_recommended, updated_at = clock_timestamp()
    where source = v_source and work_id = v_work;
  return null;
end;
$$;
create trigger update_review_stats after insert or update or delete on atsumi_community.reviews
  for each row execute function atsumi_community.update_stats();

create function public.community_v1_profile(p_nickname text default null) returns jsonb
language plpgsql security definer set search_path = '' as $$
declare v_member atsumi_community.members; v_name text;
begin
  if auth.uid() is null then
    raise exception using errcode = '28000', message = 'COMMUNITY_LOGIN_REQUIRED';
  end if;
  if p_nickname is not null then
    v_name := btrim(p_nickname);
    if char_length(v_name) not between 2 and 24 then
      raise exception using errcode = '22023', message = 'COMMUNITY_INVALID_NICKNAME';
    end if;
  end if;
  insert into atsumi_community.members(auth_user_id, nickname)
    values (auth.uid(), coalesce(v_name, '이용자-' || substr(gen_random_uuid()::text, 1, 8)))
    on conflict (auth_user_id) do nothing;
  select * into v_member from atsumi_community.members where auth_user_id = auth.uid() for update;
  if not v_member.enabled then raise exception using errcode = '42501', message = 'COMMUNITY_ACCOUNT_DISABLED'; end if;
  if v_name is not null and v_name <> v_member.nickname then
    perform atsumi_community.rate_limit(v_member.id);
    update atsumi_community.members set nickname = v_name where id = v_member.id;
    v_member.nickname := v_name;
  end if;
  return jsonb_build_object('id', v_member.id, 'nickname', v_member.nickname);
end;
$$;

create function public.community_v1_save_review(
  p_source text, p_work_id text, p_rating integer, p_recommended boolean, p_comment text, p_nickname text default null
) returns uuid language plpgsql security definer set search_path = '' as $$
declare v_member uuid; v_id uuid;
begin
  v_member := atsumi_community.require_member();
  perform atsumi_community.check_work(p_source, p_work_id);
  if p_rating is null or p_rating not between 1 and 5 or p_recommended is null
    or p_comment is null or char_length(p_comment) > 500
    or (p_nickname is not null and char_length(btrim(p_nickname)) not between 2 and 24) then
    raise exception using errcode = '22023', message = 'COMMUNITY_INVALID_REVIEW';
  end if;
  perform atsumi_community.rate_limit(v_member);
  if p_nickname is not null then
    update atsumi_community.members set nickname = btrim(p_nickname) where id = v_member;
  end if;
  insert into atsumi_community.reviews(member_id, source, work_id, rating, recommended, comment)
    values (v_member, p_source, p_work_id, p_rating, p_recommended, btrim(p_comment))
    on conflict (member_id, source, work_id) do update
      set rating = excluded.rating, recommended = excluded.recommended,
        comment = excluded.comment, updated_at = clock_timestamp()
    returning id into v_id;
  return v_id;
end;
$$;

create function public.community_v1_delete_review(p_source text, p_work_id text) returns boolean
language plpgsql security definer set search_path = '' as $$
declare v_member uuid; v_count integer;
begin
  v_member := atsumi_community.require_member();
  perform atsumi_community.check_work(p_source, p_work_id);
  perform atsumi_community.rate_limit(v_member);
  delete from atsumi_community.reviews where member_id = v_member and source = p_source and work_id = p_work_id;
  get diagnostics v_count = row_count;
  return v_count > 0;
end;
$$;

create function public.community_v1_summaries(p_works jsonb) returns jsonb
language plpgsql security definer set search_path = '' as $$
declare v_item jsonb; v_result jsonb;
begin
  if p_works is null or jsonb_typeof(p_works) <> 'array' then
    raise exception using errcode = '22023', message = 'COMMUNITY_INVALID_BATCH';
  end if;
  if jsonb_array_length(p_works) > 100 then
    raise exception using errcode = '22023', message = 'COMMUNITY_BATCH_TOO_LARGE';
  end if;
  for v_item in select value from jsonb_array_elements(p_works) loop
    perform atsumi_community.check_work(v_item->>'source', v_item->>'workId');
  end loop;
  select coalesce(jsonb_agg(jsonb_build_object(
    'source', w.source, 'workId', w.work_id, 'reviewCount', coalesce(s.review_count, 0),
    'averageRating', case when s.review_count > 0 then round(s.rating_sum::numeric / s.review_count, 2) else null end,
    'recommendationCount', coalesce(s.recommendation_count, 0)
  )), '[]'::jsonb) into v_result
  from (select distinct value->>'source' source, value->>'workId' work_id from jsonb_array_elements(p_works)) w
  left join atsumi_community.work_stats s on s.source = w.source and s.work_id = w.work_id;
  return v_result;
end;
$$;

create function public.community_v1_reviews(
  p_source text, p_work_id text, p_cursor jsonb default null
) returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_items jsonb; v_next jsonb; v_mine jsonb; v_member uuid;
begin
  perform atsumi_community.check_work(p_source, p_work_id);
  if p_cursor is not null and (p_cursor->>'createdAt' is null or p_cursor->>'id' is null) then
    raise exception using errcode = '22023', message = 'COMMUNITY_INVALID_CURSOR';
  end if;
  select id into v_member from atsumi_community.members where auth_user_id = auth.uid() and enabled;
  with page as (
    select r.*, m.nickname from atsumi_community.reviews r
    join atsumi_community.members m on m.id = r.member_id
    where r.source = p_source and r.work_id = p_work_id and not r.hidden
      and (p_cursor is null or (r.created_at, r.id) < ((p_cursor->>'createdAt')::timestamptz, (p_cursor->>'id')::uuid))
    order by r.created_at desc, r.id desc limit 21
  ), numbered as (select *, row_number() over(order by created_at desc, id desc) n from page)
  select coalesce(jsonb_agg(jsonb_build_object(
    'id', id, 'source', source, 'workId', work_id, 'nickname', nickname, 'rating', rating, 'recommended', recommended,
    'comment', comment, 'createdAt', created_at, 'updatedAt', updated_at
  ) order by n) filter(where n <= 20), '[]'::jsonb),
  case when count(*) > 20 then (jsonb_agg(jsonb_build_object('createdAt', created_at, 'id', id) order by n)->19) else null end
  into v_items, v_next from numbered;
  select jsonb_build_object('id', id, 'rating', rating, 'recommended', recommended, 'comment', comment, 'hidden', hidden)
    into v_mine from atsumi_community.reviews where member_id = v_member and source = p_source and work_id = p_work_id;
  return jsonb_build_object('items', v_items, 'nextCursor', v_next, 'mine', v_mine);
end;
$$;

-- Public reading never needs an auth session. Keyset pagination bounds each
-- response and does not re-read an expanding OFFSET on large boards.
create function public.community_v1_feed(p_source text default null, p_cursor jsonb default null)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_items jsonb; v_next jsonb;
begin
  if p_source is not null and p_source not in ('hitomi', 'danbooru') then
    raise exception 'COMMUNITY_INVALID_WORK';
  end if;
  if p_cursor is not null and (p_cursor->>'createdAt' is null or p_cursor->>'id' is null) then
    raise exception 'COMMUNITY_INVALID_CURSOR';
  end if;
  with page as (
    select r.*, m.nickname from atsumi_community.reviews r
    join atsumi_community.members m on m.id = r.member_id
    where not r.hidden and (p_source is null or r.source = p_source)
      and (p_cursor is null or (r.created_at, r.id) < ((p_cursor->>'createdAt')::timestamptz, (p_cursor->>'id')::uuid))
    order by r.created_at desc, r.id desc limit 21
  ), numbered as (select *, row_number() over(order by created_at desc, id desc) n from page)
  select coalesce(jsonb_agg(jsonb_build_object(
    'id', id, 'source', source, 'workId', work_id, 'nickname', nickname,
    'rating', rating, 'recommended', recommended, 'comment', comment,
    'createdAt', created_at, 'updatedAt', updated_at
  ) order by n) filter(where n <= 20), '[]'::jsonb),
  case when count(*) > 20 then (jsonb_agg(jsonb_build_object('createdAt', created_at, 'id', id) order by n)->19) else null end
  into v_items, v_next from numbered;
  return jsonb_build_object('items', v_items, 'nextCursor', v_next);
end;
$$;

create function public.community_v1_report(p_review_id uuid, p_reason text) returns void
language plpgsql security definer set search_path = '' as $$
declare v_member uuid;
begin
  v_member := atsumi_community.require_member();
  if p_reason is null or char_length(btrim(p_reason)) not between 1 and 300 then
    raise exception using errcode = '22023', message = 'COMMUNITY_INVALID_REPORT';
  end if;
  if not exists(select 1 from atsumi_community.reviews where id = p_review_id and not hidden and member_id <> v_member) then
    raise exception using errcode = '22023', message = 'COMMUNITY_REVIEW_NOT_REPORTABLE';
  end if;
  perform atsumi_community.rate_limit(v_member);
  insert into atsumi_community.reports(review_id, member_id, reason) values (p_review_id, v_member, btrim(p_reason))
    on conflict (review_id, member_id) do update set reason = excluded.reason;
end;
$$;

revoke all on all functions in schema atsumi_community from public, anon, authenticated;
revoke all on function public.community_v1_profile(text) from public, anon, authenticated;
revoke all on function public.community_v1_save_review(text,text,integer,boolean,text,text) from public, anon, authenticated;
revoke all on function public.community_v1_delete_review(text,text) from public, anon, authenticated;
revoke all on function public.community_v1_summaries(jsonb) from public, anon, authenticated;
revoke all on function public.community_v1_reviews(text,text,jsonb) from public, anon, authenticated;
revoke all on function public.community_v1_report(uuid,text) from public, anon, authenticated;
revoke all on function public.community_v1_feed(text,jsonb) from public, anon, authenticated;

-- Explicit grants, independent of Supabase's table/default-privilege settings.
grant execute on function public.community_v1_profile(text) to authenticated;
grant execute on function public.community_v1_save_review(text,text,integer,boolean,text,text) to authenticated;
grant execute on function public.community_v1_delete_review(text,text) to authenticated;
grant execute on function public.community_v1_report(uuid,text) to authenticated;
grant execute on function public.community_v1_summaries(jsonb) to anon, authenticated;
grant execute on function public.community_v1_reviews(text,text,jsonb) to anon, authenticated;
grant execute on function public.community_v1_feed(text,jsonb) to anon, authenticated;

notify pgrst, 'reload schema';
commit;
