-- Synthetic users/reviews are visible only inside this transaction and rolled back.
begin;
insert into auth.users(id, instance_id, aud, role, is_anonymous, created_at, updated_at)
select ('eeeeeeee-5555-4555-8555-' || lpad(n::text, 12, '0'))::uuid,
  '00000000-0000-0000-0000-000000000000', 'authenticated', 'authenticated', true, now(), now()
from generate_series(1,6) n;
insert into atsumi_community.members(auth_user_id, nickname)
select ('eeeeeeee-5555-4555-8555-' || lpad(n::text, 12, '0'))::uuid, '순위 검증-' || n
from generate_series(1,6) n;

-- Enough tied scores to verify pagination, plus duplicate-work and hidden cases.
insert into atsumi_community.reviews(member_id, source, work_id, rating, comment, created_at)
select m.id, 'hitomi', (999930000 + n)::text, 3, 'synthetic ranked review', '2000-01-01'::timestamptz
from atsumi_community.members m cross join generate_series(1,26) n
where m.auth_user_id = 'eeeeeeee-5555-4555-8555-000000000001';
insert into atsumi_community.reviews(member_id, source, work_id, rating, recommended, comment, created_at)
select id, 'hitomi', '999930101', 5, true, 'single five-star review', '2000-01-01'
from atsumi_community.members where auth_user_id = 'eeeeeeee-5555-4555-8555-000000000001';
insert into atsumi_community.reviews(member_id, source, work_id, rating, comment, created_at)
select id, 'hitomi', '999930102', 4, case when auth_user_id = 'eeeeeeee-5555-4555-8555-000000000006' then 'newest representative' else 'well-supported four-star review' end,
  '2000-01-01'::timestamptz + right(auth_user_id::text,1)::integer * interval '1 hour'
from atsumi_community.members where auth_user_id::text like 'eeeeeeee-5555-4555-8555-00000000000_';
insert into atsumi_community.reviews(member_id, source, work_id, rating, comment)
select id, 'hitomi', '999930103', 1, 'one low rating'
from atsumi_community.members where auth_user_id = 'eeeeeeee-5555-4555-8555-000000000001';
insert into atsumi_community.reviews(member_id, source, work_id, rating, comment)
select id, 'hitomi', '999930104', 1, 'three low ratings'
from atsumi_community.members where auth_user_id in ('eeeeeeee-5555-4555-8555-000000000001','eeeeeeee-5555-4555-8555-000000000002','eeeeeeee-5555-4555-8555-000000000003');
insert into atsumi_community.reviews(member_id, source, work_id, rating, comment, hidden)
select id, 'hitomi', '999930105', 5, 'hidden should never rank', true
from atsumi_community.members where auth_user_id = 'eeeeeeee-5555-4555-8555-000000000001';
insert into atsumi_community.reviews(member_id, source, work_id, rating, comment)
select id, 'danbooru', '999930102', 1, 'separate source'
from atsumi_community.members where auth_user_id = 'eeeeeeee-5555-4555-8555-000000000001';

set local role anon;
select set_config('request.jwt.claims', '{}', true);
do $$
declare mode text; page jsonb; items jsonb; cursor_value jsonb; pages integer; one_position integer; many_position integer; card jsonb;
begin
  foreach mode in array array['popular','latest','worst'] loop
    items := '[]'::jsonb; cursor_value := null; pages := 0;
    loop
      page := public.community_v1_ranked_feed('hitomi', cursor_value, mode);
      if jsonb_array_length(page->'items') > 20 then raise exception 'FAIL: response unbounded'; end if;
      if exists(select 1 from jsonb_array_elements(items) a join jsonb_array_elements(page->'items') b on a->>'id' = b->>'id') then raise exception 'FAIL: page overlap'; end if;
      items := items || (page->'items'); pages := pages + 1;
      exit when page->'nextCursor' = 'null'::jsonb;
      if pages > 100 then raise exception 'Test requires isolated DB or fewer than 2000 real reviews'; end if;
      cursor_value := page->'nextCursor';
      if cursor_value->>'order' <> mode or cursor_value->>'scope' <> 'hitomi' then raise exception 'FAIL: unscoped cursor'; end if;
    end loop;
    if pages < 2 then raise exception 'FAIL: pagination not exercised'; end if;
    if exists(select 1 from jsonb_array_elements(items) r where r->>'source' <> 'hitomi' or r->>'workId' = '999930105' or r ?| array['member_id','auth_user_id','email','access_token']) then raise exception 'FAIL: source, hidden or identity leak'; end if;
    if mode <> 'latest' then
      if (select count(*) from jsonb_array_elements(items) r where r->>'workId' = '999930102') <> 1 then raise exception 'FAIL: ranked work duplicated'; end if;
      select r into card from jsonb_array_elements(items) r where r->>'workId' = '999930102';
      if card->>'comment' <> 'newest representative' or (card#>>'{workSummary,reviewCount}')::integer <> 6 or (card#>>'{workSummary,averageRating}')::numeric <> 4 then raise exception 'FAIL: aggregate/representative'; end if;
    else
      if (select count(*) from jsonb_array_elements(items) r where r->>'workId' = '999930102') <> 6 then raise exception 'FAIL: latest must keep all reviews'; end if;
    end if;
    if mode = 'popular' then
      select ordinal into one_position from jsonb_array_elements(items) with ordinality r(item,ordinal) where item->>'workId' = '999930101';
      select ordinal into many_position from jsonb_array_elements(items) with ordinality r(item,ordinal) where item->>'workId' = '999930102';
      if one_position is null or many_position is null or many_position >= one_position then raise exception 'FAIL: rating + review-count weighting'; end if;
    elsif mode = 'worst' then
      select ordinal into one_position from jsonb_array_elements(items) with ordinality r(item,ordinal) where item->>'workId' = '999930103';
      select ordinal into many_position from jsonb_array_elements(items) with ordinality r(item,ordinal) where item->>'workId' = '999930104';
      if one_position is null or many_position is null or many_position >= one_position then raise exception 'FAIL: lowest average then review count'; end if;
    end if;
  end loop;
  page := public.community_v1_ranked_feed('hitomi', null, 'popular');
  begin
    perform public.community_v1_ranked_feed('hitomi', page->'nextCursor', 'worst');
    raise exception 'FAIL: mixed-order cursor accepted';
  exception when invalid_parameter_value then null; end;
  begin
    perform public.community_v1_ranked_feed('danbooru', page->'nextCursor', 'popular');
    raise exception 'FAIL: mixed-source cursor accepted';
  exception when invalid_parameter_value then null; end;
  begin
    perform public.community_v1_ranked_feed('hitomi', null, 'invalid');
    raise exception 'FAIL: invalid order';
  exception when invalid_parameter_value then null; end;
  begin
    perform * from atsumi_community.work_stats;
    raise exception 'FAIL: private table exposed';
  exception when insufficient_privilege then null; end;
  page := public.community_v1_feed('hitomi', null);
  if jsonb_array_length(page->'items') <> 20 then raise exception 'FAIL: older client API changed'; end if;
end $$;
reset role;
rollback;
select 'PASS: weighted popularity, worst averages, latest reviews, pagination, isolation, hidden records, no private data, old-client compatibility; synthetic data rolled back' as result;
