export type PolicyCounts = {
  allow: number;
  confirm: number;
  deny: number;
};

export type SafeConfigSummary = {
  workspaceRoot: "default" | "configured" | `workspace:${string}`;
  sandbox: {
    enabled: boolean;
    mode: "bubblewrap" | "disabled";
  };
  policyRuleCounts: PolicyCounts;
  confirmationProvider: "freedesktop" | "unavailable" | "unknown";
};

export type Capabilities = {
  sessions: boolean;
  confirmation: boolean;
  notificationActions: boolean;
};

export type AgentRegistryEntry = {
  agentId: string;
  displayName: string;
  enabled: boolean;
  secretHash: string;
  doName: string;
  lastSeenAt?: string;
  capabilities: Capabilities;
};

export type TaskStatus =
  | "queued"
  | "running"
  | "waiting_confirmation"
  | "completed"
  | "failed"
  | "rejected";

export type TaskResult = {
  agentId: string;
  taskId: string;
  status: TaskStatus;
  exitCode: number | null;
  stdoutTail: string;
  stderrTail: string;
  truncated: boolean;
  rejectReason?: string;
  startedAt: string;
  updatedAt: string;
};

export type SessionState =
  | "running"
  | "exited"
  | "killed"
  | "failed"
  | "waiting_confirmation";

export type SessionInfo = {
  agentId: string;
  sessionId: string;
  state: SessionState;
  program: string;
  args: string[];
  commandPreview: string;
  startedAt: string;
  updatedAt: string;
  exitCode: number | null;
  stdoutTail?: string;
  stderrTail?: string;
  truncated?: boolean;
  rejectReason?: string;
};

export type ExecElement = {
  program: string;
  args: string[];
};

export type ExecRequest = ExecElement & {
  agentId: string;
  needConfirm: boolean;
};

export type BatchExecRequest = {
  agentId: string;
  elements: ExecElement[];
  needConfirm: boolean;
};

export type StartSessionRequest = ExecRequest;
