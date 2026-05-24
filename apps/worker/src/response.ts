export function json(data: unknown, init: ResponseInit = {}): Response {
  return Response.json(data, {
    ...init,
    headers: {
      "content-type": "application/json; charset=utf-8",
      ...init.headers
    }
  });
}

export function error(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, { status });
}
