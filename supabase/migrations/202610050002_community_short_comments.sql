-- Limit future writes only. Existing longer reviews remain readable and removable.
-- CREATE OR REPLACE keeps the existing function grants and owner-only write path.
create or replace function public.community_v1_save_review(
  p_source text, p_work_id text, p_rating integer, p_recommended boolean, p_comment text, p_nickname text default null
) returns uuid language plpgsql security definer set search_path = '' as $$
declare v_member uuid; v_id uuid;
begin
  v_member := atsumi_community.require_member();
  perform atsumi_community.check_work(p_source, p_work_id);
  if p_rating is null or p_rating not between 1 and 5 or p_recommended is null
    or p_comment is null or char_length(p_comment) > 100
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
