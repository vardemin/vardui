package dev.vard.app

import dev.vard.host.VardActivity

/**
 * The whole app: everything (project loading, edge-to-edge, predictive back, push, modules)
 * lives in [VardActivity]. Override `modules()` to add app-specific native modules.
 */
class MainActivity : VardActivity()
