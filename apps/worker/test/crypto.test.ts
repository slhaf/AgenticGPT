import { describe, expect, it } from "vitest";
import { isAuthorizedActionRequest, parseBearerToken } from "../src/auth";
import { constantTimeEqual } from "../src/crypto";

describe("constantTimeEqual", () => {
  it("compares equal and unequal values", () => {
    expect(constantTimeEqual("abc", "abc")).toBe(true);
    expect(constantTimeEqual("abc", "abd")).toBe(false);
    expect(constantTimeEqual("abc", "abcd")).toBe(false);
  });
});

describe("action auth parsing", () => {
  it("parses bearer tokens case-insensitively and trims token whitespace", () => {
    expect(parseBearerToken("Bearer abc123")).toBe("abc123");
    expect(parseBearerToken("bearer   abc123   ")).toBe("abc123");
    expect(parseBearerToken("Basic abc123")).toBeNull();
  });

  it("authorizes only the token body", () => {
    expect(isAuthorizedActionRequest("Bearer abc123", "abc123")).toBe(true);
    expect(isAuthorizedActionRequest("bearer   abc123   ", "abc123")).toBe(true);
    expect(isAuthorizedActionRequest("Bearer Bearer abc123", "abc123")).toBe(false);
    expect(isAuthorizedActionRequest("", "abc123")).toBe(false);
  });
});
