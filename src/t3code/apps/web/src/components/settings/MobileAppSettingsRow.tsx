import { T, useGT } from "gt-react";

import { MOBILE_APP_URL } from "../../branding";
import { Button } from "../ui/button";
import { Popover, PopoverPopup, PopoverTitle, PopoverTrigger } from "../ui/popover";
import { QRCodeSvg } from "../ui/qr-code";
import { SettingsRow } from "./settingsLayout";
import { useSearchableSetting } from "./settingsSearch";

export function MobileAppSettingsRow() {
  const searchableSetting = useSearchableSetting();
  const gt = useGT();
  if (!MOBILE_APP_URL) {
    return null;
  }
  return (
    <SettingsRow
      {...searchableSetting("mobile-app")}
      description={gt("Connect to your server from your phone or tablet.")}
      control={
        <Popover>
          <PopoverTrigger render={<Button size="sm" variant="outline" />}>
            <T>Get the mobile app</T>
          </PopoverTrigger>
          <PopoverPopup align="end">
            <div className="flex flex-col items-center gap-3">
              <PopoverTitle>
                <T>Mobile app</T>
              </PopoverTitle>
              <p className="text-xs text-muted-foreground">
                <T>Scan with your phone camera.</T>
              </p>
              <div className="rounded-lg bg-white p-1.5">
                <QRCodeSvg
                  value={MOBILE_APP_URL}
                  size={128}
                  level="M"
                  marginSize={1}
                  title={gt("VikEA mobile download link")}
                />
              </div>
              <Button
                size="sm"
                variant="outline"
                render={<a href={MOBILE_APP_URL} target="_blank" rel="noreferrer noopener" />}
              >
                <T>Open download page</T>
              </Button>
            </div>
          </PopoverPopup>
        </Popover>
      }
    />
  );
}
