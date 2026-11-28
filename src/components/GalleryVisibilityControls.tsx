import { exploreVisibilityLabels, type ExploreVisibility } from "../state/exploreVisibility";

export type GalleryVisibilityControlsProps = {
  visibility: ExploreVisibility;
  onChange: (key: keyof ExploreVisibility, checked: boolean) => void;
};

export function GalleryVisibilityControls({ visibility, onChange, label }: GalleryVisibilityControlsProps & { label: string }) {
  return <div className="explore-visibility" role="group" aria-label={label}>
    {(Object.keys(exploreVisibilityLabels) as (keyof ExploreVisibility)[]).map(key => (
      <label key={key}><input type="checkbox" checked={visibility[key]}
        onChange={event => onChange(key, event.currentTarget.checked)} />{exploreVisibilityLabels[key]}</label>
    ))}
  </div>;
}
