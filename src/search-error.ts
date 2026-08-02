export interface SearchErrorPresentation {
  readonly title: string;
  readonly message: string;
}

const errorKind = (error: unknown): string => {
  if (typeof error !== "object" || error === null) return "";

  const taggedError = error as { readonly _tag?: unknown; readonly name?: unknown };
  if (typeof taggedError._tag === "string") return taggedError._tag;
  return typeof taggedError.name === "string" ? taggedError.name : "";
};

export const presentSearchError = (error: unknown): SearchErrorPresentation => {
  switch (errorKind(error)) {
    case "ApiTransportError":
      return {
        title: "Connection problem",
        message: "Unable to reach search. Check your connection and try again.",
      };
    case "ApiDecodeError":
      return {
        title: "Search response error",
        message: "Search returned an unexpected response. Try again in a moment.",
      };
    case "ApiResponseError":
      return {
        title: "Search unavailable",
        message: "Search returned an error. Try again in a moment.",
      };
    default:
      return {
        title: "Search failed",
        message: "Unable to search. Try again.",
      };
  }
};
