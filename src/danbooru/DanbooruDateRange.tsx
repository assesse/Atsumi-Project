import { useLayoutEffect, useRef } from "react";
import { FluentIcon } from "../components/FluentIcon";
import { isDanbooruDate } from "./searchPreferences";
import "./DanbooruDateRange.css";

type Props = { from: string; to: string; onChange: (from: string, to: string) => void };

/** Segmented typing plus the native calendar, without an extra calendar Tab stop. */
export function DanbooruDateRange({ from, to, onChange }: Props) {
  const inputs = useRef<Array<HTMLInputElement | null>>([]);
  const calendars = useRef<Array<HTMLInputElement | null>>([]);
  const openCalendar = (index: number) => {
    const input = calendars.current[index];
    if (!input) return;
    if (typeof input.showPicker === "function") input.showPicker();
    else input.click();
  };
  const pastedFocus = useRef<number | null>(null);
  useLayoutEffect(() => {
    if (pastedFocus.current === null) return;
    const index = pastedFocus.current;
    pastedFocus.current = null;
    // Commit the pasted date before blurring a partially entered month/day.
    inputs.current[index]?.focus();
  }, [from, to]);
  const moveNext = (index: number) => inputs.current[index + 1]?.focus();
  const update = (dateIndex: number, partIndex: number, raw: string) => {
    const current: [string, string] = [from, to];
    const parts = (current[dateIndex] || "--").split("-");
    parts[partIndex] = raw.replace(/\D/g, "").slice(0, partIndex === 0 ? 4 : 2);
    current[dateIndex] = parts.every(part => !part) ? "" : parts.join("-");
    onChange(current[0], current[1]);
    if (parts[partIndex].length === (partIndex === 0 ? 4 : 2)) moveNext(dateIndex * 3 + partIndex);
  };
  return <div className="danbooru-date-range">
    {[from, to].map((value, dateIndex) => <div className="danbooru-date-field" key={dateIndex} role="group" aria-label={`Danbooru 등록 ${dateIndex === 0 ? "시작일" : "종료일"}`}>
      <span>{dateIndex === 0 ? "시작" : "종료"}</span>
      <div className="danbooru-date-segments">
        {["연도", "월", "일"].map((label, partIndex) => <input
          key={label}
          ref={node => { inputs.current[dateIndex * 3 + partIndex] = node; }}
          type="text" inputMode="numeric" autoComplete="off"
          aria-label={`${dateIndex === 0 ? "시작" : "종료"} ${label}`}
          placeholder={partIndex === 0 ? "YYYY" : partIndex === 1 ? "MM" : "DD"}
          maxLength={partIndex === 0 ? 4 : 2}
          value={value.split("-")[partIndex] ?? ""}
          onFocus={event => event.currentTarget.select()}
          onKeyDown={event => { if (event.altKey && event.key === "ArrowDown") { event.preventDefault(); openCalendar(dateIndex); } }}
          onChange={event => update(dateIndex, partIndex, event.target.value)}
          onBlur={event => {
            if (partIndex === 0 || event.target.value.length !== 1) return;
            const current: [string, string] = [from, to];
            const parts = (current[dateIndex] || "--").split("-");
            parts[partIndex] = event.target.value.padStart(2, "0");
            current[dateIndex] = parts.join("-");
            onChange(current[0], current[1]);
          }}
          onPaste={event => {
            const text = event.clipboardData.getData("text").trim();
            const digits = text.replace(/[-./\s]/g, "");
            if (!/^\d{8}$/.test(digits)) return;
            event.preventDefault();
            const current: [string, string] = [from, to];
            const pasted = `${digits.slice(0, 4)}-${digits.slice(4, 6)}-${digits.slice(6, 8)}`;
            if (current[dateIndex] === pasted) {
              inputs.current[dateIndex === 0 ? 3 : 5]?.focus();
              return;
            }
            current[dateIndex] = pasted;
            pastedFocus.current = dateIndex === 0 ? 3 : 5;
            onChange(current[0], current[1]);
          }}
        />)}
        <span className="danbooru-calendar-control">
          <button type="button" className="danbooru-calendar-button" tabIndex={-1} aria-label={`${dateIndex === 0 ? "시작일" : "종료일"} 달력 열기`} title="달력 선택 (Alt+↓)" onClick={() => openCalendar(dateIndex)}><FluentIcon glyph="\uE787" /></button>
          <input
            ref={node => { calendars.current[dateIndex] = node; }}
            type="date" tabIndex={-1} className="danbooru-native-calendar" aria-label={`${dateIndex === 0 ? "시작일" : "종료일"} 달력`}
            min="0001-01-01" max="9999-12-31" value={isDanbooruDate(value) ? value : ""}
            onChange={event => {
              const current: [string, string] = [from, to];
              current[dateIndex] = event.target.value;
              onChange(current[0], current[1]);
            }}
          />
        </span>
      </div>
    </div>)}
  </div>;
}
