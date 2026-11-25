import type { ColorValue } from "react-native";
import Svg, { Path } from "react-native-svg";
import { withUniwind } from "uniwind";

const ThemedPath = withUniwind(Path);

export function VikEAWordmark(props: {
  readonly height: number;
  readonly color?: ColorValue;
  readonly colorClassName?: string;
}) {
  const aspectRatio = 1;
  return (
    <Svg
      accessibilityLabel="VikEA"
      height={props.height}
      width={props.height * aspectRatio}
      viewBox="0 0 24 24"
    >
      <ThemedPath
        d="M3 4h4l5 12 5-12h4l-7.5 16h-3L3 4Z"
        color={props.color}
        colorClassName={props.colorClassName}
        fill="currentColor"
      />
    </Svg>
  );
}
