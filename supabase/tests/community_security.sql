-- Real PostgreSQL regression checks. All users/reviews below are synthetic;
-- the transaction is always rolled back and publishes no test reviews.
begin;
insert into auth.users(id, instance_id, aud, role, is_anonymous, created_at, updated_at)
values ('aaaaaaaa-1111-4111-8111-111111111111', '00000000-0000-0000-0000-000000000000', 'authenticated', 'authenticated', true, now(), now()),
       ('bbbbbbbb-2222-4222-8222-222222222222', '00000000-0000-0000-0000-000000000000', 'authenticated', 'authenticated', true, now(), now());

set local role anon;
select set_config('request.jwt.claims', '{}', true);
do $$ begin
  perform public.community_v1_feed();
  perform public.community_v1_reviews('hitomi', '999900001');
  begin
    perform public.community_v1_profile();
    raise exception 'FAIL: unauthenticated profile allowed';
  exception when insufficient_privilege then null; end;
  begin
    perform public.community_v1_save_review('hitomi','999900001',5,true,'test',null);
    raise exception 'FAIL: unauthenticated write allowed';
  exception when insufficient_privilege then null; end;
  begin
    perform 1 from atsumi_community.members;
    raise exception 'FAIL: private members exposed';
  exception when insufficient_privilege then null; end;
end $$;
reset role;

set local role authenticated;
select set_config('request.jwt.claims', '{"sub":"aaaaaaaa-1111-4111-8111-111111111111","role":"authenticated","is_anonymous":true}', true);
select public.community_v1_profile('테스트 작성자 A');
select public.community_v1_save_review('hitomi','999900001',4,true,'최초 후기','테스트 작성자 A');
do $$ begin
  begin
    perform public.community_v1_save_review('hitomi','999900002',5,false,'빠른 요청',null);
    raise exception 'FAIL: write throttle missing';
  exception when raise_exception then
    if sqlerrm <> 'COMMUNITY_WRITE_TOO_FAST' then raise; end if;
  end;
end $$;
reset role;
update atsumi_community.members set last_write_at = now() - interval '1 minute'
  where auth_user_id = 'aaaaaaaa-1111-4111-8111-111111111111';
set local role authenticated;
select public.community_v1_save_review('hitomi','999900001',5,false,'수정 후기','수정된 닉네임');
reset role;
do $$ declare s atsumi_community.work_stats; begin
  select * into s from atsumi_community.work_stats where source = 'hitomi' and work_id = '999900001';
  if s.review_count <> 1 or s.rating_sum <> 5 or s.recommendation_count <> 0 then raise exception 'FAIL: upsert summary'; end if;
end $$;

set local role authenticated;
select set_config('request.jwt.claims', '{"sub":"bbbbbbbb-2222-4222-8222-222222222222","role":"authenticated","is_anonymous":true}', true);
select public.community_v1_profile('테스트 작성자 B');
do $$ begin
  if public.community_v1_delete_review('hitomi','999900001') then raise exception 'FAIL: deleting other author'; end if;
  if public.community_v1_reviews('hitomi','999900001')->'mine' <> 'null'::jsonb then raise exception 'FAIL: other review marked mine'; end if;
end $$;
reset role;
update atsumi_community.members set last_write_at = null where auth_user_id = 'bbbbbbbb-2222-4222-8222-222222222222';
set local role authenticated;
select public.community_v1_save_review('hitomi','999900001',1,true,'다른 작성자',null);
reset role;
do $$ declare s atsumi_community.work_stats; begin
  select * into s from atsumi_community.work_stats where source = 'hitomi' and work_id = '999900001';
  if s.review_count <> 2 or s.rating_sum <> 6 or s.recommendation_count <> 1 then raise exception 'FAIL: independent authors'; end if;
end $$;

-- Moderation remains authoritative even if the owner edits a hidden review.
update atsumi_community.reviews set hidden = true where member_id = (select id from atsumi_community.members where auth_user_id = 'aaaaaaaa-1111-4111-8111-111111111111');
update atsumi_community.members set last_write_at = null where auth_user_id = 'aaaaaaaa-1111-4111-8111-111111111111';
set local role authenticated;
select set_config('request.jwt.claims', '{"sub":"aaaaaaaa-1111-4111-8111-111111111111","role":"authenticated","is_anonymous":true}', true);
select public.community_v1_save_review('hitomi','999900001',3,true,'숨긴 후기 수정',null);
do $$ begin
  if public.community_v1_reviews('hitomi','999900001')->'mine'->>'hidden' <> 'true' then raise exception 'FAIL: hidden review resurrected'; end if;
end $$;
reset role;
do $$ declare s atsumi_community.work_stats; begin
  select * into s from atsumi_community.work_stats where source = 'hitomi' and work_id = '999900001';
  if s.review_count <> 1 or s.rating_sum <> 1 or s.recommendation_count <> 1 then raise exception 'FAIL: hidden summary'; end if;
end $$;

-- Large enough for pagination. The work key isolates fixtures from real data.
insert into atsumi_community.reviews(member_id, source, work_id, rating, comment)
select id, 'danbooru', (999910000 + n)::text, 3, 'synthetic pagination'
from atsumi_community.members cross join generate_series(1,25) n
where auth_user_id = 'bbbbbbbb-2222-4222-8222-222222222222';
set local role anon;
select set_config('request.jwt.claims', '{}', true);
do $$ declare page jsonb; next_page jsonb; row jsonb; begin
  page := public.community_v1_feed('danbooru');
  if jsonb_array_length(page->'items') <> 20 or page->'nextCursor' = 'null'::jsonb then raise exception 'FAIL: bounded pagination'; end if;
  next_page := public.community_v1_feed('danbooru', page->'nextCursor');
  if exists(select 1 from jsonb_array_elements(page->'items') a join jsonb_array_elements(next_page->'items') b on a->>'id' = b->>'id') then raise exception 'FAIL: overlapping pages'; end if;
  for row in select value from jsonb_array_elements(page->'items') loop
    if row ?| array['auth_user_id','member_id','email','access_token','refresh_token'] then raise exception 'FAIL: private fields leaked'; end if;
  end loop;
  page := public.community_v1_reviews('hitomi', '999900001');
  if jsonb_array_length(page->'items') <> 1 or page->'mine' <> 'null'::jsonb then raise exception 'FAIL: hidden/mine public exposure'; end if;
end $$;
reset role;

-- Invalid input and daily cap are enforced on the server, not just by the UI.
update atsumi_community.members set last_write_at = null, daily_writes = 100, write_day = current_date
 where auth_user_id = 'bbbbbbbb-2222-4222-8222-222222222222';
set local role authenticated;
select set_config('request.jwt.claims', '{"sub":"bbbbbbbb-2222-4222-8222-222222222222","role":"authenticated","is_anonymous":true}', true);
do $$ begin
  begin
    perform public.community_v1_save_review('hitomi','999900001',0,false,'bad',null);
    raise exception 'FAIL: invalid rating allowed';
  exception when invalid_parameter_value then null; end;
  begin
    perform public.community_v1_save_review('hitomi','999900001',5,false,'limit',null);
    raise exception 'FAIL: daily limit missing';
  exception when raise_exception then if sqlerrm <> 'COMMUNITY_DAILY_LIMIT' then raise; end if; end;
end $$;
reset role;
update atsumi_community.members set last_write_at = null, daily_writes = 0 where auth_user_id = 'bbbbbbbb-2222-4222-8222-222222222222';
set local role authenticated;
select public.community_v1_delete_review('hitomi','999900001');
reset role;
do $$ declare s atsumi_community.work_stats; begin
  select * into s from atsumi_community.work_stats where source = 'hitomi' and work_id = '999900001';
  if s.review_count <> 0 or s.rating_sum <> 0 or s.recommendation_count <> 0 then raise exception 'FAIL: delete summary'; end if;
end $$;
rollback;
select 'PASS: public read, anonymous author, private tables, ownership, moderation, stats, pagination, rate limits; synthetic data rolled back' as result;
