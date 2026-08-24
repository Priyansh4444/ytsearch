import type { Component } from "solid-js";
import type { JSX } from "@solidjs/web";

type IconProps = Omit<JSX.SvgSVGAttributes<SVGSVGElement>, "children"> & {
  size?: number;
};

const Icon: Component<IconProps & { children: JSX.Element }> = (props) => (
  <svg
    xmlns="http://www.w3.org/2000/svg"
    width={props.size ?? 24}
    height={props.size ?? 24}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
    class={props.class}
    aria-hidden={props["aria-hidden"]}
  >
    {props.children}
  </svg>
);

// Path data comes from Lucide's ISC-licensed icon set.
export const CheckIcon: Component<IconProps> = (props) => (
  <Icon {...props}>
    <path d="M20 6 9 17l-5-5" />
  </Icon>
);

export const ChevronLeftIcon: Component<IconProps> = (props) => (
  <Icon {...props}>
    <path d="m15 18-6-6 6-6" />
  </Icon>
);

export const CopyIcon: Component<IconProps> = (props) => (
  <Icon {...props}>
    <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
    <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
  </Icon>
);

export const ChevronDownIcon: Component<IconProps> = (props) => (
  <Icon {...props}>
    <path d="m6 9 6 6 6-6" />
  </Icon>
);

export const LoaderCircleIcon: Component<IconProps> = (props) => (
  <Icon {...props}>
    <path d="M21 12a9 9 0 1 1-6.219-8.56" />
  </Icon>
);

export const PlayIcon: Component<IconProps> = (props) => (
  <Icon {...props}>
    <path d="m9 18 6-6-6-6v12Z" />
  </Icon>
);

export const ThumbsDownIcon: Component<IconProps> = (props) => (
  <Icon {...props}>
    <path d="M9 18.12 10 14H4.17a2 2 0 0 1-1.92-2.56l2.33-8A2 2 0 0 1 6.5 2H20a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-2.76a2 2 0 0 0-1.79 1.11L12 22a3.13 3.13 0 0 1-3-3.88Z" />
    <path d="M17 14V2" />
  </Icon>
);

export const ThumbsUpIcon: Component<IconProps> = (props) => (
  <Icon {...props}>
    <path d="M15 5.88 14 10h5.83a2 2 0 0 1 1.92 2.56l-2.33 8A2 2 0 0 1 17.5 22H4a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h2.76a2 2 0 0 0 1.79-1.11L12 2a3.13 3.13 0 0 1 3 3.88Z" />
    <path d="M7 10v12" />
  </Icon>
);
