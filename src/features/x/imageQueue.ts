// X-only admission: queued images leave with their viewport/account, and the
// native bridge never accumulates thousands of waiting image requests.
let workers = 0;
const queue: (() => void)[] = [];
export function scheduleXImage(task: () => Promise<void>): (() => void) | undefined {
  const start = () => {
    workers++;
    void Promise.resolve().then(task).catch(() => {}).finally(() => {
      workers--;
      queue.shift()?.();
    });
  };
  if (workers < 2) start();
  else if (queue.length < 64) queue.push(start);
  else return undefined;
  return () => { const index = queue.indexOf(start); if (index >= 0) queue.splice(index, 1); };
}
