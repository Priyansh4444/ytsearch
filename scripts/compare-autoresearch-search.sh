#!/usr/bin/env sh
set -eu

BASELINE="${1:-}"
CANDIDATE="${2:-}"
MIN_RUNS="${MIN_RUNS:-3}"
CONFIDENCE_Z="${CONFIDENCE_Z:-1.96}"

if [ ! -f "$BASELINE" ] || [ ! -f "$CANDIDATE" ]; then
  echo "usage: $0 BASELINE_LOG CANDIDATE_LOG" >&2
  exit 2
fi

awk -v min_runs="$MIN_RUNS" -v z="$CONFIDENCE_Z" '
  function field(name,    i,pair) {
    for (i=1; i<=NF; i++) {
      split($i, pair, "=")
      if (pair[1] == name) return pair[2]
    }
    return ""
  }
  function seconds(value) { sub(/s$/, "", value); return value + 0 }
  function add(mode, case_name, value, videos, moments, http,    key,n) {
    key=mode SUBSEP case_name
    seen_case[case_name]=1
    if (http != "200") errors[key]++
    if (http != "200") return
    n=++count[key]
    values[key,n]=value
    sum[key]+=value
    sumsq[key]+=value*value
    signature=videos "/" moments
    if (!(key in corpus)) corpus[key]=signature
    else if (corpus[key] != signature) unstable[key]=1
  }
  function sort_key(key,n,    i,j,tmp) {
    for (i=1;i<=n;i++) sorted[i]=values[key,i]
    for (i=2;i<=n;i++) {
      tmp=sorted[i]; j=i-1
      while (j>=1 && sorted[j]>tmp) { sorted[j+1]=sorted[j]; j-- }
      sorted[j+1]=tmp
    }
  }
  FNR==NR {
    case_name=field("case"); total=field("total")
    if (case_name!="" && total!="") add("baseline",case_name,seconds(total),field("total_videos"),field("total_moments"),field("http"))
    next
  }
  {
    case_name=field("case"); total=field("total")
    if (case_name!="" && total!="") add("candidate",case_name,seconds(total),field("total_videos"),field("total_moments"),field("http"))
  }
  END {
    print "case\tbaseline_ms\tcandidate_ms\tdelta_pct\tbaseline_p95_ms\tcandidate_p95_ms\tconfidence_margin_ms\tverdict"
    overall="keep"
    for (case_name in seen_case) {
      bk="baseline" SUBSEP case_name; ck="candidate" SUBSEP case_name
      bn=count[bk]+0; cn=count[ck]+0
      verdict="keep"
      if (bn<min_runs || cn<min_runs || errors[bk] || errors[ck] || unstable[bk] || unstable[ck] || corpus[bk]!=corpus[ck]) verdict="invalid"
      bm=bn ? sum[bk]/bn : 0; cm=cn ? sum[ck]/cn : 0
      bv=bn>1 ? (sumsq[bk]-(sum[bk]*sum[bk]/bn))/(bn-1) : 0
      cv=cn>1 ? (sumsq[ck]-(sum[ck]*sum[ck]/cn))/(cn-1) : 0
      margin=z*sqrt((bn ? bv/bn : 0)+(cn ? cv/cn : 0))
      delete sorted; sort_key(bk,bn); bp95=sorted[int((bn*0.95)+0.999999)]
      delete sorted; sort_key(ck,cn); cp95=sorted[int((cn*0.95)+0.999999)]
      if (verdict=="keep" && !(cm+margin<bm && cp95<=bp95)) verdict="discard"
      delta=bm ? ((cm-bm)/bm)*100 : 0
      printf "%s\t%.0f\t%.0f\t%+.1f%%\t%.0f\t%.0f\t%.0f\t%s\n", case_name,bm*1000,cm*1000,delta,bp95*1000,cp95*1000,margin*1000,verdict
      if (verdict=="invalid") overall="invalid"
      else if (verdict=="discard" && overall!="invalid") overall="discard"
    }
    print "overall\t" overall
    if (overall=="invalid") exit 2
    if (overall=="discard") exit 1
  }
' "$BASELINE" "$CANDIDATE"
