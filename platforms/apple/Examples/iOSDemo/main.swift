// The smallest iOS app around a Vard project: VardViewController over the bundled `app/`.
// Built without an Xcode project by ../../run-ios-sim.sh.
import UIKit
import VardHost

final class AppDelegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?

    func application(_ app: UIApplication, didFinishLaunchingWithOptions o: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        window = UIWindow(frame: UIScreen.main.bounds)
        window?.rootViewController = VardViewController()
        window?.makeKeyAndVisible()
        return true
    }

    func application(_ app: UIApplication, open url: URL, options: [UIApplication.OpenURLOptionsKey: Any] = [:]) -> Bool {
        (window?.rootViewController as? VardViewController)?.lifecycle.open(url)
        return true
    }

    func application(_ app: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken t: Data) {
        NotificationsModule.setPushToken(t)
    }
}

UIApplicationMain(CommandLine.argc, CommandLine.unsafeArgv, nil, NSStringFromClass(AppDelegate.self))
