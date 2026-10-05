-- Public ranking over existing public ratings only. No new identity or vote data.
-- Applied 2026-10-05; community_ranked_feed.sql passed on PostgreSQL (ROLLBACK).
-- Keep community_v1_feed(text,jsonb) intact for older desktop versions.
begin;

create index if not exists work_stats_popular on atsumi_community.work_stats
  (source, (round((rating_sum + 15)::numeric / (review_count + 5), 8)) desc, review_count desc, work_id desc)
  where review_count > 0;
create index if not exists work_stats_worst on atsumi_community.work_stats
  (source, ((-round(rating_sum::numeric / nullif(review_count, 0), 8))) desc, review_count desc, work_id desc)
  where review_count > 0;

create or replace function public.community_v1_ranked_feed(
  p_source text default null, p_cursor jsonb default null, p_order text default 'latest'
) returns jsonb language plpgsql security definer set search_path = '' as $$
declare v_page jsonb; v_items jsonb; v_next jsonb;
begin
  if p_source is not null and p_source not in ('hitomi', 'danbooru') then
    raise exception using errcode = '22023', message = 'COMMUNITY_INVALID_WORK';
  end if;
  if p_order is null or p_order not in ('popular', 'latest', 'worst') then
    raise exception using errcode = '22023', message = 'COMMUNITY_INVALID_ORDER';
  end if;
  if p_cursor is not null and (
    jsonb_typeof(p_cursor) is distinct from 'object'
    or p_cursor->>'order' is distinct from p_order
    or p_cursor->'scope' is distinct from coalesce(to_jsonb(p_source), 'null'::jsonb)
  ) then
    raise exception using errcode = '22023', message = 'COMMUNITY_INVALID_CURSOR';
  end if;
  if p_order = 'latest' then
    v_page := public.community_v1_feed(p_source, p_cursor);
    if v_page->'nextCursor' <> 'null'::jsonb then
      v_page := jsonb_set(v_page, '{nextCursor}', v_page->'nextCursor' || jsonb_build_object('order', p_order, 'scope', p_source));
    end if;
    return v_page;
  end if;
  if p_cursor is not null then
    if coalesce(p_cursor->>'score', '') !~ '^-?[0-9]{1,12}(\.[0-9]{1,8})?$'
      or coalesce(p_cursor->>'reviewCount', '') !~ '^[1-9][0-9]{0,15}$'
      or coalesce(p_cursor->>'source', '') not in ('hitomi', 'danbooru')
      or coalesce(p_cursor->>'workId', '') !~ '^[1-9][0-9]{0,19}$'
      or (p_source is not null and p_cursor->>'source' <> p_source) then
      raise exception using errcode = '22023', message = 'COMMUNITY_INVALID_CURSOR';
    end if;
  end if;

  -- Bayesian average: a neutral 3-star / 5-review prior. A single 5-star
  -- review does not outrank a well-supported 4-star work. Worst uses the raw
  -- work average, with more reviews first when averages tie.
  with popular as (
    select s.*, round((rating_sum + 15)::numeric / (review_count + 5), 8) as score
    from atsumi_community.work_stats s
    where p_order = 'popular' and review_count > 0 and (p_source is null or source = p_source)
      and (p_cursor is null or
        (round((rating_sum + 15)::numeric / (review_count + 5), 8), review_count, source, work_id)
        < ((p_cursor->>'score')::numeric, (p_cursor->>'reviewCount')::bigint, p_cursor->>'source', p_cursor->>'workId'))
    order by score desc, review_count desc, source desc, work_id desc limit 21
  ), worst as (
    select s.*, -round(rating_sum::numeric / nullif(review_count, 0), 8) as score
    from atsumi_community.work_stats s
    where p_order = 'worst' and review_count > 0 and (p_source is null or source = p_source)
      and (p_cursor is null or
        (-round(rating_sum::numeric / nullif(review_count, 0), 8), review_count, source, work_id)
        < ((p_cursor->>'score')::numeric, (p_cursor->>'reviewCount')::bigint, p_cursor->>'source', p_cursor->>'workId'))
    order by score desc, review_count desc, source desc, work_id desc limit 21
  ), ranked as (
    select * from popular union all select * from worst
  ), numbered as (
    select s.*, r.id, r.rating, r.recommended, r.comment, r.created_at, r.updated_at as review_updated_at, m.nickname,
      row_number() over(order by s.score desc, s.review_count desc, s.source desc, s.work_id desc) as n
    from ranked s
    cross join lateral (
      select * from atsumi_community.reviews r where r.source = s.source and r.work_id = s.work_id and not r.hidden
      order by r.created_at desc, r.id desc limit 1
    ) r
    join atsumi_community.members m on m.id = r.member_id
  )
  select coalesce(jsonb_agg(jsonb_build_object(
    'id', id, 'source', source, 'workId', work_id, 'nickname', nickname,
    'rating', rating, 'recommended', recommended, 'comment', comment,
    'createdAt', created_at, 'updatedAt', review_updated_at,
    'workSummary', jsonb_build_object('reviewCount', review_count, 'averageRating', round(rating_sum::numeric / review_count, 2))
  ) order by n) filter(where n <= 20), '[]'::jsonb),
  case when count(*) > 20 then (jsonb_agg(jsonb_build_object(
    'order', p_order, 'scope', p_source, 'score', score::text, 'reviewCount', review_count, 'source', source, 'workId', work_id
  ) order by n)->19) else null end
  into v_items, v_next from numbered;
  return jsonb_build_object('items', v_items, 'nextCursor', v_next);
end;
$$;

revoke all on function public.community_v1_ranked_feed(text,jsonb,text) from public, anon, authenticated;
grant execute on function public.community_v1_ranked_feed(text,jsonb,text) to anon, authenticated;
notify pgrst, 'reload schema';
commit;
