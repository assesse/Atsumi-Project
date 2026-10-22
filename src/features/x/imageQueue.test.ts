import { expect, it, vi } from "vitest";
import { scheduleXImage } from "./imageQueue";
const drain=async()=>{for(let i=0;i<12;i++)await Promise.resolve();};
it("limits admission, removes cancelled queue entries, and bounds waiting work",async()=>{
  const finish:(()=>void)[]=[];
  const running=vi.fn(()=>new Promise<void>(resolve=>finish.push(resolve)));
  scheduleXImage(running);scheduleXImage(running);
  const cancelled=vi.fn(async()=>{}), cancel=scheduleXImage(cancelled)!;
  const pending=vi.fn(async()=>{}), releases=Array.from({length:63},()=>scheduleXImage(pending)!);
  expect(scheduleXImage(pending)).toBeUndefined();
  cancel();releases.forEach(release=>release());
  await drain();expect(running).toHaveBeenCalledTimes(2);expect(pending).not.toHaveBeenCalled();
  finish.forEach(resolve=>resolve());await drain();expect(cancelled).not.toHaveBeenCalled();
  scheduleXImage(pending);await drain();expect(pending).toHaveBeenCalledTimes(1);
});
