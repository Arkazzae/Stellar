import { describe, expect, it } from "vitest";

import { singleQueryValue } from "./query";

const read = (query: string, name: string) => singleQueryValue(new URLSearchParams(query), name);

describe("singleQueryValue", () => {
  it("returns a parameter the address names once", () => {
    expect(read("token=mailed-token", "token")).toBe("mailed-token");
    expect(read("result=linked&other=1", "result")).toBe("linked");
  });

  it("decodes the value as the address carries it", () => {
    expect(read("token=a%2Bb%3D%3D", "token")).toBe("a+b==");
  });

  it("keeps an empty value, which the pages read as no token", () => {
    expect(read("token=", "token")).toBe("");
  });

  it("reads a missing parameter as absent", () => {
    expect(read("", "token")).toBeNull();
    expect(read("other=1", "token")).toBeNull();
  });

  it("reads a repeated parameter as absent instead of choosing one", () => {
    expect(read("token=one&token=two", "token")).toBeNull();
    expect(read("result=linked&result=linked", "result")).toBeNull();
  });
});
