// Use the VikEA production mark across all channels.
import appIconUrl from "../../../../assets/prod/vikea-web-apple-touch-180.png";

export function VikEAAppIcon(props: { readonly className?: string }) {
  return <img src={appIconUrl} alt="" className={props.className} />;
}
