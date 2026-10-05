-- Owner-only, bounded review history. No identity creation or public owner IDs.
-- Applied to Atsumi Community (yfpgshvflnawmrimyfzo), 2026-10-05 KST.
-- supabase/tests/community_my_reviews.sql passed on the server; fixtures rolled back.
begin;

create index if not exists reviews_member_recent
  on atsumi_community.reviews(member_id, created_at desc, id desc);

create or replace function public.community_v1_my_reviews(p_cursor jsonb default null)
returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_member uuid; v_profile jsonb; v_items jsonb; v_next jsonb;
begin
  v_member := atsumi_community.require_member();
  if p_cursor is not null and (jsonb_typeof(p_cursor) <> 'object'
      or p_cursor->>'createdAt' is null or p_cursor->>'id' is null) then
    raise exception using errcode = '22023', message = 'COMMUNITY_INVALID_CURSOR';
  end if;
  select jsonb_build_object('id', id, 'nickname', nickname) into v_profile
    from atsumi_community.members where id = v_member;
  with page as (
    select r.*, m.nickname from atsumi_community.reviews r
    join atsumi_community.members m on m.id = r.member_id
    where r.member_id = v_member
      and (p_cursor is null or (r.created_at, r.id) < ((p_cursor->>'createdAt')::timestamptz, (p_cursor->>'id')::uuid))
    order by r.created_at desc, r.id desc limit 21
  ), numbered as (select *, row_number() over(order by created_at desc, id desc) n from page)
  select coalesce(jsonb_agg(jsonb_build_object(
    'id', id, 'source', source, 'workId', work_id, 'nickname', nickname,
    'rating', rating, 'recommended', recommended, 'comment', comment,
    'hidden', hidden, 'createdAt', created_at, 'updatedAt', updated_at
  ) order by n) filter(where n <= 20), '[]'::jsonb),
  case when count(*) > 20 then (jsonb_agg(jsonb_build_object('createdAt', created_at, 'id', id) order by n)->19) else null end
  into v_items, v_next from numbered;
  return jsonb_build_object('profile', v_profile, 'identityIssued', true, 'items', v_items, 'nextCursor', v_next);
end;
$$;

revoke all on function public.community_v1_my_reviews(jsonb) from public, anon, authenticated;
grant execute on function public.community_v1_my_reviews(jsonb) to authenticated;
notify pgrst, 'reload schema';
commit;
