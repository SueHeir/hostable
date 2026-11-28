# Native phone app (later)

The same web client wrapped with Capacitor. Nothing here is built yet; the
config is ready for when a store app is wanted:

```sh
cd client && npm run build
npm install --save-dev @capacitor/cli @capacitor/core @capacitor/ios @capacitor/android
npx cap init --web-dir dist   # uses capacitor/capacitor.config.json's values
npx cap add ios && npx cap add android
npx cap sync
```

Point the app at the live site by setting `server.url` to
`https://dinkydelvers.com/play/` (or the launch domain) in the config. Check
`package-lock.json` with `scripts/ip_check.sh` after installing.
