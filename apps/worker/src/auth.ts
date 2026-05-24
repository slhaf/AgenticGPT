import { constantTimeEqual } from "./crypto";

export function parseBearerToken(authorization: string): string | null {
  const match = authorization.match(/^Bearer\s+(.+)$/i);
  return match?.[1]?.trim() || null;
}

export function isAuthorizedActionRequest(authorization: string, apiKey: string): boolean {
  const token = parseBearerToken(authorization);
  return !!token && constantTimeEqual(token, apiKey.trim());
}
