export class ApiTransportError extends Error {
  readonly _tag = "ApiTransportError";

  constructor(
    readonly operation: string,
    readonly cause: unknown,
  ) {
    super(`${operation} failed${cause instanceof Error ? `: ${cause.message}` : ""}`);
    this.name = "ApiTransportError";
  }
}

export class ApiResponseError extends Error {
  readonly _tag = "ApiResponseError";

  constructor(
    message: string,
    readonly status: number,
    readonly code?: string,
    readonly availableAt?: string,
  ) {
    super(message);
    this.name = "ApiResponseError";
  }
}

export class ApiDecodeError extends Error {
  readonly _tag = "ApiDecodeError";

  constructor(
    readonly operation: string,
    readonly status: number,
    readonly cause: unknown,
  ) {
    super(`${operation} (HTTP ${status})`);
    this.name = "ApiDecodeError";
  }
}

export type ApiError = ApiTransportError | ApiResponseError | ApiDecodeError;
