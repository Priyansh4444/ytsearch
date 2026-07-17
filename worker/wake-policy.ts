const INDEX_REQUEST_APPROVAL_PATH = /^\/api\/index-requests\/[^/]+\/approve$/;

export function shouldWakeIndexer(method: string, pathname: string, upstreamOk: boolean): boolean {
  return method === "POST" && INDEX_REQUEST_APPROVAL_PATH.test(pathname) && upstreamOk;
}
