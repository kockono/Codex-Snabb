# Image Viewer Specification

## Purpose

Enables viewing of image files (JPG, PNG, GIF, WEBP, BMP) directly in the editor area of Codex-Snabb using terminal image protocols, strictly adhering to CPU and RAM budgets.

## ADDED Requirements

### Requirement: Supported Extensions
The system MUST recognize `.jpg`, `.jpeg`, `.png`, `.gif`, `.webp`, and `.bmp` files (case-insensitive).

#### Scenario: Opening supported image
- GIVEN an image file with a supported extension
- WHEN the user opens the file from explorer, quick open, or CLI
- THEN the system opens the file in an image viewer tab instead of a text editor tab

#### Scenario: Opening text file
- GIVEN a file with a non-image extension (e.g., `.txt`, `.rs`)
- WHEN the user opens the file
- THEN the system opens the file in a standard text editor tab

### Requirement: Tab Display and Read-Only State
The system MUST display the file name in the tab without a "modified" indicator. The image tab MUST be read-only and ignore standard editor keybindings.

#### Scenario: Modifying an image tab
- GIVEN an active image viewer tab
- WHEN the user presses editor keybindings (e.g., typing, undo)
- THEN the keybindings are ignored and the image is unmodified

### Requirement: Terminal Protocol Detection
The system MUST detect the terminal's image protocol capabilities in order of preference: Kitty > Sixel > Half-blocks. Detection MUST be lazy, occurring only on the first image open.

#### Scenario: Terminal with Kitty graphics support
- GIVEN a terminal that supports the Kitty image protocol
- WHEN the user opens an image
- THEN the system renders the image using the Kitty protocol

#### Scenario: Fallback terminal
- GIVEN a terminal without advanced graphic protocol support
- WHEN the user opens an image
- THEN the system renders the image using half-blocks fallback

### Requirement: Async Decoding and Downscaling
The system MUST decode images asynchronously using `spawn_blocking` to prevent blocking the event loop. Images larger than 1920x1080 MUST be downscaled prior to decoding.

#### Scenario: Opening a large image
- GIVEN an image larger than 1920x1080
- WHEN the user opens the image
- THEN the system downscales it before decoding to keep RAM usage below budget
- AND the event loop remains responsive (under 16ms input-to-render)

### Requirement: Error Handling
The system MUST display a clear error message in the editor area if the image fails to decode.

#### Scenario: Opening a corrupt image
- GIVEN a corrupt or invalid image file
- WHEN the user opens the image
- THEN the system displays a readable error message in the editor area instead of crashing

### Requirement: Image Scaling
The system MUST scale the image to fit the available editor area while preserving its aspect ratio.

#### Scenario: Resizing the terminal
- GIVEN an active image viewer tab
- WHEN the user resizes the terminal window
- THEN the system re-scales the image to fit the new area while preserving aspect ratio

### Requirement: Memory Management and Cache
The system MUST cache the pre-encoded image protocol data per tab. Zero allocations in the render loop. RAM MUST only be consumed by the active tab's image; inactive image tabs MUST release their memory.

#### Scenario: Switching tabs
- GIVEN an active image tab and an inactive image tab
- WHEN the user switches from the image tab to a text tab
- THEN the image tab's memory is released, and the text editor operates normally without artifacts

#### Scenario: Closing an image tab
- GIVEN an active image tab
- WHEN the user closes the tab
- THEN all memory associated with that image is freed