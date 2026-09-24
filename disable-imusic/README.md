# Disable iMusic

A tiny macOS menu bar app that stops Apple Music from launching. By default, pressing Play on your keyboard with no other player open makes macOS start Music. This app force-quits Music the instant it starts. It leaves every other app alone, so media keys keep working with Spotify, browsers, VLC, and so on.

## How it works

It watches for app launches through `NSWorkspace` and force-quits `com.apple.Music` (and the old `com.apple.iTunes`). There's no private API and no system changes. Music can flash in the Dock for a split second before it gets killed.

## Requirements

- macOS 14 or later
- Xcode (or the Swift 6 toolchain)

## Build and install

```sh
./build.sh            # builds build/DisableIMusic.app
./build.sh --install  # also copies it into /Applications
open /Applications/DisableIMusic.app
```

The settings window opens on first launch.

## Options

You get the same options from the menu bar icon and from the settings window:

- **Block Apple Music**: turns blocking on or off.
- **Show menu bar icon**: turn this off to run with no UI at all.
- **Launch at login**: registers the app as a login item.

**Getting the icon back:** open the app again from Spotlight or Finder. The settings window will appear.

## Test

```sh
swift test
```

## Uninstall

Turn off "Launch at login," quit the app, then delete `/Applications/DisableIMusic.app`. To also clear its settings, run `defaults delete dev.ai-slop.disable-imusic`.
