import { describe, expect, it } from "vitest";
import { normalizeSoopChannel, soopChannelUrl, soopPlayerUrl } from "./soopLinks";

describe("SOOP channel links", () => {
  it.each(["aflol", " AfLol ", "https://play.sooplive.com/aflol/12345", "play.sooplive.co.kr/aflol/embed", "http://play.afreecatv.com/aflol/123/embed", "https://bj.afreecatv.com/aflol", "https://ch.sooplive.com/aflol", "https://play.sooplive.com/aflol?from=share#player"])("normalizes %s", (input: string) => {
    expect(normalizeSoopChannel(input)).toBe("aflol");
  });
  it.each(["", "https://play.sooplive.com.evil.test/aflol", "https://play.sooplive.com@evil.test/aflol", "https://user@play.sooplive.com/aflol", "https://play.sooplive.com:444/aflol", "https://play.sooplive.com/aflol/abc", "https://ch.sooplive.com/aflol/posts", "file:///aflol", "javascript:alert(1)", "https://play.sooplive.com/aflol/embed/embed"])("rejects %s", (input: string) => {
    expect(normalizeSoopChannel(input)).toBeNull();
  });
  it("constructs only canonical first-party player URLs", () => {
    expect(soopPlayerUrl("AfLol")).toBe("https://play.sooplive.com/aflol/embed");
    expect(soopChannelUrl("AfLol")).toBe("https://play.sooplive.com/aflol");
    expect(() => soopPlayerUrl("https://evil.test/")).toThrow();
  });
  it("accepts the current official station address including its board tab", () => {
    expect(normalizeSoopChannel("https://www.sooplive.com/station/aflol/board")).toBe("aflol");
    expect(normalizeSoopChannel("https://www.sooplive.co.kr/station/aflol")).toBe("aflol");
    expect(normalizeSoopChannel("https://www.sooplive.com/unrelated/aflol")).toBeNull();
  });
});
