-- Real PostgreSQL checks; synthetic authors/reviews never leave this transaction.
begin;
insert into auth.users(id, instance_id, aud, role, is_anonymous, created_at, updated_at)
values ('cccccccc-3333-4333-8333-333333333333', '00000000-0000-0000-0000-000000000000', 'authenticated', 'authenticated', true, now(), now()),
       ('dddddddd-4444-4444-8444-444444444444', '00000000-0000-0000-0000-000000000000', 'authenticated', 'authenticated', true, now(), now());
insert into atsumi_community.members(auth_user_id, nickname)
values ('cccccccc-3333-4333-8333-333333333333', '동일 닉네임'), ('dddddddd-4444-4444-8444-444444444444', '동일 닉네임');
insert into atsumi_community.reviews(member_id, source, work_id, rating, comment, hidden)
select id, case when n % 2 = 0 then 'hitomi' else 'danbooru' end, (999920000 + n)::text, 4, 'synthetic own history', n = 1
from atsumi_community.members cross join generate_series(1,25) n
where auth_user_id = 'cccccccc-3333-4333-8333-333333333333';
insert into atsumi_community.reviews(member_id, source, work_id, rating, comment)
select id, 'hitomi', '999929999', 2, 'synthetic other author'
from atsumi_community.members where auth_user_id = 'dddddddd-4444-4444-8444-444444444444';

set local role anon;
select set_config('request.jwt.claims', '{}', true);
do $$ begin
  begin
    perform public.community_v1_my_reviews();
    raise exception 'FAIL: anonymous owner-history access';
  exception when insufficient_privilege then null; end;
end $$;
reset role;

set local role authenticated;
select set_config('request.jwt.claims', '{"sub":"cccccccc-3333-4333-8333-333333333333","role":"authenticated"}', true);
do $$ declare page jsonb; following jsonb; all_items jsonb; item jsonb; begin
  page := public.community_v1_my_reviews();
  if jsonb_array_length(page->'items') <> 20 or page->'nextCursor' = 'null'::jsonb then raise exception 'FAIL: first page bound'; end if;
  following := public.community_v1_my_reviews(page->'nextCursor');
  if jsonb_array_length(following->'items') <> 5 or following->'nextCursor' <> 'null'::jsonb then raise exception 'FAIL: next page bound'; end if;
  if exists(select 1 from jsonb_array_elements(page->'items') a join jsonb_array_elements(following->'items') b on a->>'id' = b->>'id') then raise exception 'FAIL: overlapping pages'; end if;
  all_items := (page->'items') || (following->'items');
  if not exists(select 1 from jsonb_array_elements(all_items) r where r->>'hidden' = 'true') then raise exception 'FAIL: own hidden review missing'; end if;
  if (select count(distinct r->>'source') from jsonb_array_elements(all_items) r) <> 2 then raise exception 'FAIL: both sources missing'; end if;
  for item in select value from jsonb_array_elements(all_items) loop
    if item->>'comment' <> 'synthetic own history' then raise exception 'FAIL: another author leaked'; end if;
    if item ?| array['auth_user_id','member_id','email','access_token','refresh_token'] then raise exception 'FAIL: credential fields leaked'; end if;
  end loop;
  if page->'profile' ?| array['auth_user_id','email','access_token','refresh_token'] then raise exception 'FAIL: private profile fields leaked'; end if;
  begin
    perform public.community_v1_my_reviews('{}');
    raise exception 'FAIL: invalid cursor accepted';
  exception when invalid_parameter_value then null; end;
end $$;
reset role;

-- An authenticated JWT without a registered author cannot enumerate members.
set local role authenticated;
select set_config('request.jwt.claims', '{}', true);
do $$ begin
  begin
    perform public.community_v1_my_reviews();
    raise exception 'FAIL: missing JWT accepted';
  exception when invalid_authorization_specification then null; end;
end $$;
reset role;
update atsumi_community.members set enabled = false where auth_user_id = 'cccccccc-3333-4333-8333-333333333333';
set local role authenticated;
select set_config('request.jwt.claims', '{"sub":"cccccccc-3333-4333-8333-333333333333","role":"authenticated"}', true);
do $$ begin
  begin
    perform public.community_v1_my_reviews();
    raise exception 'FAIL: disabled author accepted';
  exception when insufficient_privilege then null; end;
end $$;
reset role;
rollback;
select 'PASS: own history, same-nickname isolation, hidden reviews, bounded pagination, no secrets, authentication and disabled-author checks; synthetic data rolled back' as result;
