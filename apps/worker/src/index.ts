import { DurableObject } from "cloudflare:workers";
import { isAuthorizedActionRequest, parseBearerToken } from "./auth";
import { constantTimeEqual, sha256Hex } from "./crypto";
import { error, json } from "./response";
import type {
  AgentRegistryEntry,
  BatchExecRequest,
  BatchExecResult,
  ExecRequest,
  SafeConfigSummary,
  SessionInfo,
  StartSessionRequest,
  TaskResult
} from "./types";

export interface Env {
  API_KEY: string;
  AGENT_REGISTRY: KVNamespace;
  AGENT_DO: DurableObjectNamespace<AgentObject>;
}

type AgentEnvelope =
  | { type: "exec"; requestId: string; taskId: string; payload: ExecRequest }
  | { type: "batchExec"; requestId: string; taskId: string; payload: BatchExecRequest }
  | { type: "startSession"; requestId: string; sessionId: string; payload: StartSessionRequest }
  | { type: "listSessions"; requestId: string }
  | { type: "inspectSession"; requestId: string; sessionId: string }
  | { type: "waitSession"; requestId: string; sessionId: string; seconds: number }
  | { type: "killSession"; requestId: string; sessionId: string };

type AgentMessage =
  | { type: "hello"; configSummary?: SafeConfigSummary }
  | { type: "heartbeat"; sentAt: string }
  | { type: "task_update"; task: TaskResult }
  | { type: "session_update"; session: SessionInfo }
  | { type: "response"; requestId: string; data: unknown };

const EXEC_RESPONSE_TIMEOUT_MS = 35_000;
const MAX_WAIT_SECONDS = 30;

function nowIso(): string {
  return new Date().toISOString();
}

function randomId(prefix: string): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  const body = [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${prefix}_${body}`;
}

function commandPreview(program: string, args: string[]): string {
  return [program, ...args].map((part) => (/\s/.test(part) ? JSON.stringify(part) : part)).join(" ");
}

function validateProgramArgs(value: unknown): value is { program: string; args: string[] } {
  if (!value || typeof value !== "object") return false;
  const candidate = value as { program?: unknown; args?: unknown };
  return (
    typeof candidate.program === "string" &&
    candidate.program.length > 0 &&
    Array.isArray(candidate.args) &&
    candidate.args.every((arg) => typeof arg === "string")
  );
}

async function readJson(request: Request): Promise<unknown> {
  try {
    return await request.json();
  } catch {
    return undefined;
  }
}

async function requireActionAuth(request: Request, env: Env): Promise<Response | null> {
  const authorization = request.headers.get("authorization") || "";
  if (!isAuthorizedActionRequest(authorization, env.API_KEY)) {
    const token = parseBearerToken(authorization);
    console.warn("action auth failed", {
      hasAuthorization: authorization.length > 0,
      scheme: authorization.split(/\s+/, 1)[0] || null,
      tokenLength: token?.length || 0,
      expectedLength: env.API_KEY.trim().length
    });
    return error(401, "unauthorized", "Invalid GPT Actions API key");
  }
  return null;
}

async function getRegistry(env: Env, agentId: string): Promise<AgentRegistryEntry | null> {
  if (!/^[a-zA-Z0-9._:-]{1,80}$/.test(agentId)) return null;
  return await env.AGENT_REGISTRY.get<AgentRegistryEntry>(`agent:${agentId}`, "json");
}

async function requireAgent(env: Env, agentId: string): Promise<AgentRegistryEntry | Response> {
  const entry = await getRegistry(env, agentId);
  if (!entry || !entry.enabled) {
    return error(404, "agent_not_found", "Agent is not registered or enabled");
  }
  return entry;
}

function getStub(env: Env, entry: AgentRegistryEntry): DurableObjectStub<AgentObject> {
  return env.AGENT_DO.getByName(entry.doName || `agent:${entry.agentId}`);
}

async function listAgents(env: Env): Promise<Response> {
  const listed = await env.AGENT_REGISTRY.list({ prefix: "agent:" });
  const agents = [];
  for (const key of listed.keys) {
    const entry = await env.AGENT_REGISTRY.get<AgentRegistryEntry>(key.name, "json");
    if (!entry || !entry.enabled) continue;
    const stub = getStub(env, entry);
    const status = await stub.getStatus();
    agents.push({
      agentId: entry.agentId,
      displayName: entry.displayName,
      online: status.online,
      lastSeenAt: status.lastSeenAt || entry.lastSeenAt || null,
      capabilities: entry.capabilities,
      configSummary: status.configSummary
    });
  }
  return json({ agents });
}

async function parseExecRequest(request: Request): Promise<ExecRequest | Response> {
  const body = await readJson(request);
  if (!body || typeof body !== "object") return error(400, "invalid_request", "Expected JSON object");
  const value = body as Partial<ExecRequest>;
  if (
    typeof value.program !== "string" ||
    value.program.length === 0 ||
    !Array.isArray(value.args) ||
    !value.args.every((arg) => typeof arg === "string") ||
    typeof value.agentId !== "string" ||
    typeof value.needConfirm !== "boolean"
  ) {
    return error(400, "invalid_request", "Expected agentId, program, args, and needConfirm");
  }
  return {
    agentId: value.agentId,
    program: value.program,
    args: value.args,
    needConfirm: value.needConfirm
  };
}

async function parseBatchRequest(request: Request): Promise<BatchExecRequest | Response> {
  const body = await readJson(request);
  if (!body || typeof body !== "object") return error(400, "invalid_request", "Expected JSON object");
  const value = body as Partial<BatchExecRequest>;
  if (
    typeof value.agentId !== "string" ||
    typeof value.needConfirm !== "boolean" ||
    !Array.isArray(value.elements) ||
    value.elements.length === 0 ||
    !value.elements.every(validateProgramArgs)
  ) {
    return error(400, "invalid_request", "Expected agentId, elements, and needConfirm");
  }
  return {
    agentId: value.agentId,
    elements: value.elements,
    needConfirm: value.needConfirm
  };
}

async function handleConnect(request: Request, env: Env, agentId: string): Promise<Response> {
  const entry = await getRegistry(env, agentId);
  if (!entry || !entry.enabled) return error(404, "agent_not_found", "Agent is not registered or enabled");

  const secret = request.headers.get("x-agent-secret") || "";
  const secretHash = await sha256Hex(secret);
  if (!constantTimeEqual(secretHash, entry.secretHash)) {
    return error(401, "unauthorized_agent", "Invalid agent secret");
  }

  const updated = { ...entry, lastSeenAt: nowIso() };
  await env.AGENT_REGISTRY.put(`agent:${agentId}`, JSON.stringify(updated));
  return getStub(env, entry).fetch(request);
}

async function route(request: Request, env: Env): Promise<Response> {
  const url = new URL(request.url);

  const connectMatch = url.pathname.match(/^\/v1\/agents\/([^/]+)\/connect$/);
  if (connectMatch && request.headers.get("upgrade")?.toLowerCase() === "websocket") {
    return handleConnect(request, env, decodeURIComponent(connectMatch[1]));
  }

  const auth = await requireActionAuth(request, env);
  if (auth) return auth;

  if (request.method === "GET" && url.pathname === "/v1/agents") return listAgents(env);

  if (request.method === "POST" && url.pathname === "/v1/exec") {
    const parsed = await parseExecRequest(request);
    if (parsed instanceof Response) return parsed;
    const entry = await requireAgent(env, parsed.agentId);
    if (entry instanceof Response) return entry;
    return json(await getStub(env, entry).exec(parsed));
  }

  if (request.method === "POST" && url.pathname === "/v1/batchExec") {
    const parsed = await parseBatchRequest(request);
    if (parsed instanceof Response) return parsed;
    const entry = await requireAgent(env, parsed.agentId);
    if (entry instanceof Response) return entry;
    return json(await getStub(env, entry).batchExec(parsed));
  }

  const taskMatch = url.pathname.match(/^\/v1\/tasks\/([^/]+)$/);
  if (request.method === "GET" && taskMatch) {
    const agentId = url.searchParams.get("agentId") || "";
    const entry = await requireAgent(env, agentId);
    if (entry instanceof Response) return entry;
    const result = await getStub(env, entry).getTask(decodeURIComponent(taskMatch[1]));
    return result ? json(result) : error(404, "task_not_found", "Task was not found");
  }

  if (request.method === "POST" && url.pathname === "/v1/sessions/start") {
    const parsed = await parseExecRequest(request);
    if (parsed instanceof Response) return parsed;
    const entry = await requireAgent(env, parsed.agentId);
    if (entry instanceof Response) return entry;
    return json(await getStub(env, entry).startSession(parsed));
  }

  if (request.method === "GET" && url.pathname === "/v1/sessions") {
    const agentId = url.searchParams.get("agentId") || "";
    const entry = await requireAgent(env, agentId);
    if (entry instanceof Response) return entry;
    return json({ sessions: await getStub(env, entry).listSessions() });
  }

  const sessionMatch = url.pathname.match(/^\/v1\/sessions\/([^/]+)(?:\/(wait|kill))?$/);
  if (sessionMatch) {
    const sessionId = decodeURIComponent(sessionMatch[1]);
    const action = sessionMatch[2];
    const agentId = url.searchParams.get("agentId") || "";
    const entry = await requireAgent(env, agentId);
    if (entry instanceof Response) return entry;
    const stub = getStub(env, entry);
    if (!action && request.method === "GET") {
      const session = await stub.inspectSession(sessionId);
      return session ? json(session) : error(404, "session_not_found", "Session was not found");
    }
    if (action === "wait" && request.method === "POST") {
      const body = await readJson(request);
      const seconds = Math.min(
        MAX_WAIT_SECONDS,
        Math.max(0, typeof (body as { seconds?: unknown })?.seconds === "number" ? (body as { seconds: number }).seconds : 0)
      );
      return json(await stub.waitSession(sessionId, seconds));
    }
    if (action === "kill" && request.method === "POST") {
      return json(await stub.killSession(sessionId));
    }
  }

  return error(404, "not_found", "Route not found");
}

export default {
  fetch: route
} satisfies ExportedHandler<Env>;

export class AgentObject extends DurableObject<Env> {
  private socket: WebSocket | null = null;
  private lastSeenAt: string | null = null;
  private configSummary: SafeConfigSummary = {
    workspaceRoot: "configured",
    sandbox: { enabled: false, mode: "disabled" },
    policyRuleCounts: { allow: 0, confirm: 0, deny: 0 },
    confirmationProvider: "unknown"
  };
  private tasks = new Map<string, TaskResult>();
  private sessions = new Map<string, SessionInfo>();
  private pending = new Map<string, (data: unknown) => void>();

  constructor(state: DurableObjectState, env: Env) {
    super(state, env);
  }

  async fetch(request: Request): Promise<Response> {
    if (request.headers.get("upgrade")?.toLowerCase() !== "websocket") {
      return error(426, "websocket_required", "Agent connections require WebSocket");
    }
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair) as [WebSocket, WebSocket];
    this.socket?.close(1012, "Replaced by a newer connection");
    this.socket = server;
    this.ctx.acceptWebSocket(server);
    this.lastSeenAt = nowIso();
    return new Response(null, { status: 101, webSocket: client });
  }

  webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): void {
    if (typeof message !== "string") return;
    let parsed: AgentMessage;
    try {
      parsed = JSON.parse(message) as AgentMessage;
    } catch {
      return;
    }
    this.lastSeenAt = nowIso();
    if (parsed.type === "hello") {
      if (parsed.configSummary) this.configSummary = parsed.configSummary;
      return;
    }
    if (parsed.type === "heartbeat") {
      ws.send(JSON.stringify({ type: "heartbeat_ack", sentAt: parsed.sentAt, receivedAt: nowIso() }));
      return;
    }
    if (parsed.type === "task_update") {
      this.tasks.set(parsed.task.taskId, parsed.task);
      return;
    }
    if (parsed.type === "session_update") {
      this.sessions.set(parsed.session.sessionId, parsed.session);
      return;
    }
    if (parsed.type === "response") {
      const resolve = this.pending.get(parsed.requestId);
      if (resolve) {
        this.pending.delete(parsed.requestId);
        resolve(parsed.data);
      }
    }
  }

  webSocketClose(): void {
    this.socket = null;
  }

  webSocketError(): void {
    this.socket = null;
  }

  private currentSocket(): WebSocket | null {
    return this.socket ?? this.ctx.getWebSockets()[0] ?? null;
  }

  getStatus(): { online: boolean; lastSeenAt: string | null; configSummary: SafeConfigSummary } {
    return {
      online: this.ctx.getWebSockets().length > 0,
      lastSeenAt: this.lastSeenAt,
      configSummary: this.configSummary
    };
  }

  async exec(payload: ExecRequest): Promise<TaskResult> {
    const taskId = randomId("task");
    const queued = this.createTask(payload.agentId, taskId);
    this.tasks.set(taskId, queued);

    if (!this.currentSocket()) {
      return queued;
    }

    const requestId = randomId("req");
    const data = await this.requestAgent({ type: "exec", requestId, taskId, payload }, EXEC_RESPONSE_TIMEOUT_MS);
    if (data) {
      const result = data as TaskResult;
      this.tasks.set(taskId, result);
      return result;
    }

    const timeoutResult: TaskResult = {
      ...queued,
      status: "timeout",
      rejectReason: "exec_timeout_use_session",
      updatedAt: nowIso()
    };
    this.tasks.set(taskId, timeoutResult);
    return timeoutResult;
  }

  async batchExec(payload: BatchExecRequest): Promise<BatchExecResult> {
    const batchId = randomId("batch");

    if (!this.currentSocket()) {
      const at = nowIso();
      return {
        agentId: payload.agentId,
        batchId,
        status: "partial_failed",
        results: payload.elements.map((element, index) => ({
          index,
          program: element.program,
          args: element.args,
          result: {
            agentId: payload.agentId,
            taskId: `${batchId}:element:${index}`,
            status: "failed",
            exitCode: null,
            stdoutTail: "",
            stderrTail: "",
            truncated: false,
            rejectReason: "agent_offline",
            startedAt: at,
            updatedAt: at
          }
        })),
        startedAt: at,
        updatedAt: at
      };
    }

    const requestId = randomId("req");
    const data = await this.requestAgent({ type: "batchExec", requestId, taskId: batchId, payload }, EXEC_RESPONSE_TIMEOUT_MS);
    if (data) {
      return data as BatchExecResult;
    }

    const at = nowIso();
    return {
      agentId: payload.agentId,
      batchId,
      status: "timeout",
      results: payload.elements.map((element, index) => ({
        index,
        program: element.program,
        args: element.args,
        result: {
          agentId: payload.agentId,
          taskId: `${batchId}:element:${index}`,
          status: "timeout",
          exitCode: null,
          stdoutTail: "",
          stderrTail: "",
          truncated: false,
          rejectReason: "exec_timeout_use_session",
          startedAt: at,
          updatedAt: at
        }
      })),
      startedAt: at,
      updatedAt: at
    };
  }

  getTask(taskId: string): TaskResult | null {
    return this.tasks.get(taskId) || null;
  }

  async startSession(payload: StartSessionRequest): Promise<{ status: string; sessionId: string }> {
    const sessionId = randomId("sess");
    const at = nowIso();
    this.sessions.set(sessionId, {
      agentId: payload.agentId,
      sessionId,
      state: payload.needConfirm ? "waiting_confirmation" : "running",
      program: payload.program,
      args: payload.args,
      commandPreview: commandPreview(payload.program, payload.args),
      startedAt: at,
      updatedAt: at,
      exitCode: null
    });
    await this.sendCommand({ type: "startSession", requestId: randomId("req"), sessionId, payload });
    return { status: "started", sessionId };
  }

  async listSessions(): Promise<SessionInfo[]> {
    await this.requestAgent({ type: "listSessions", requestId: randomId("req") }, 1_000);
    return [...this.sessions.values()].filter((session) => session.state === "running" || session.state === "waiting_confirmation");
  }

  async inspectSession(sessionId: string): Promise<SessionInfo | null> {
    await this.requestAgent({ type: "inspectSession", requestId: randomId("req"), sessionId }, 1_000);
    return this.sessions.get(sessionId) || null;
  }

  async waitSession(sessionId: string, seconds: number): Promise<SessionInfo | { status: "offline"; sessionId: string }> {
    const data = await this.requestAgent(
      { type: "waitSession", requestId: randomId("req"), sessionId, seconds },
      Math.min(MAX_WAIT_SECONDS * 1_000, seconds * 1_000 + 1_000)
    );
    return (data as SessionInfo | undefined) || this.sessions.get(sessionId) || { status: "offline", sessionId };
  }

  async killSession(sessionId: string): Promise<SessionInfo | { status: "offline"; sessionId: string }> {
    const data = await this.requestAgent({ type: "killSession", requestId: randomId("req"), sessionId }, 2_000);
    return (data as SessionInfo | undefined) || this.sessions.get(sessionId) || { status: "offline", sessionId };
  }

  private createTask(agentId: string, taskId: string): TaskResult {
    const at = nowIso();
    const online = this.currentSocket() !== null;
    return {
      agentId,
      taskId,
      status: online ? "queued" : "failed",
      exitCode: null,
      stdoutTail: "",
      stderrTail: "",
      truncated: false,
      rejectReason: online ? undefined : "agent_offline",
      startedAt: at,
      updatedAt: at
    };
  }

  private async sendCommand(command: AgentEnvelope): Promise<void> {
    const socket = this.currentSocket();
    if (!socket) return;
    socket.send(JSON.stringify(command));
  }

  private async requestAgent(command: AgentEnvelope, timeoutMs: number): Promise<unknown | undefined> {
    const socket = this.currentSocket();
    if (!socket) return undefined;
    const result = new Promise<unknown | undefined>((resolve) => {
      const timer = setTimeout(() => {
        this.pending.delete(command.requestId);
        resolve(undefined);
      }, timeoutMs);
      this.pending.set(command.requestId, (data) => {
        clearTimeout(timer);
        resolve(data);
      });
    });
    socket.send(JSON.stringify(command));
    return result;
  }

}
