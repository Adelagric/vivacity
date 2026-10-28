<?php
namespace Neos\ComposerPlugin;

use Composer\Package\PackageInterface;
use Composer\Installer\InstallerInterface;
use Composer\Installer\LibraryInstaller;

/**
 * Custom installer for flow packages.
 *
 */
class Installer extends LibraryInstaller implements InstallerInterface
{

    const PATTERN_MATCH_PACKAGEKEY = '/^[a-z0-9]+\.(?:[a-z0-9][\.a-z0-9]*)+$/i';

    /**
     * Allowed package type prefixes for valid flow packages.
     *
     * @var array
     */
    protected $allowedPackageTypePrefixes = array('typo3-flow-', 'neos-');

    /**
     * Flow package type to path mapping templates.
     *
     * @var array
     */
    protected $packageTypeToPathMapping = array(
        'plugin' => 'Packages/Plugins/{flowPackageName}',
        'site' => 'Packages/Sites/{flowPackageName}',
        'boilerplate' => 'Packages/Boilerplates/{flowPackageName}',
        'build' => 'Build/{flowPackageName}',
        'package' => 'Packages/Application/{flowPackageName}',
        'package-collection' => 'Packages/{flowPackageName}',
        '*' => 'Packages/{camelCasedType}/{flowPackageName}'
    );

    /**
     * Decides if the installer supports the given type
     *
     * @param  string $packageType
     * @return boolean
     */
    public function supports($packageType)
    {
        if ($this->getFlowPackageType($packageType) === false) {
            return false;
        }

        return true;
    }

    /**
     * Returns the installation path of a package
     *
     * @param  PackageInterface $package
     * @return string path to install the packgae in
     */
    public function getInstallPath(PackageInterface $package)
    {
        $flowPackageType = $this->getFlowPackageType($package->getType());
        $camelCasedType = $this->camelCaseFlowPackageType($flowPackageType);
        $flowPackageName = $this->deriveFlowPackageName($package);

        if (isset($this->packageTypeToPathMapping[$flowPackageType])) {
            $installPath = $this->packageTypeToPathMapping[$flowPackageType];
        } else {
            $installPath = $this->packageTypeToPathMapping['*'];
        }

        return $this->replacePlaceholdersInPath($installPath, compact('flowPackageType', 'camelCasedType', 'flowPackageName'));
    }

    /**
     * Camel case the flow package type.
     * "framework" => "Framework"
     * "some-project" => "SomeProject"
     *
     * @param string $flowPackageType
     * @return string
     */
    protected function camelCaseFlowPackageType($flowPackageType)
    {
        $packageTypeParts = explode('-', $flowPackageType);
        $packageTypeParts = array_map('ucfirst', $packageTypeParts);
        return implode('', $packageTypeParts);
    }

    /**
     * Replace placeholders in the install path.
     *
     * @param string $path
     * @param array $arguments
     * @return string
     */
    protected function replacePlaceholdersInPath($path, $arguments)
    {
        foreach ($arguments as $argumentName => $argumentValue) {
            $path = str_replace('{' . $argumentName . '}', $argumentValue, $path);
        }

        return $path;
    }

    /**
     * Gets the Flow package type based on the given composer package type. "typo3-flow-framework" would return "framework".
     * Returns FALSE if the given composerPackageType is not a Flow package type.
     *
     * @param string $composerPackageType
     * @return bool|string
     */
    protected function getFlowPackageType($composerPackageType)
    {
        foreach ($this->allowedPackageTypePrefixes as $allowedPackagePrefix) {
            $packagePrefixPosition = strpos($composerPackageType, $allowedPackagePrefix);
            if ($packagePrefixPosition === 0) {
                return substr($composerPackageType, strlen($allowedPackagePrefix));
            }
        }

        return false;
    }

    /**
     * Find the correct Flow package name for the given package.
     * Will try the following order:
     *
     * - composer manifest "extras.installer-name"
     * - first PSR-0 autoloading namespace
     * - first PSR-4 autoloading namespace
     * - composer manifest "extras.package-key"
     * - composer package name (Does not work in all cases but common cases should be fine. Eg. "foo/bar" => "Foo.Bar", "foo/bar-baz" => "Foo.Bar.Baz")
     *
     * @param PackageInterface $package
     * @return string
     */
    protected function deriveFlowPackageName(PackageInterface $package)
    {
        $extras = $package->getExtra();
        $autoload = $package->getAutoload();
        if (isset($extras['installer-name'])) {
            $flowPackageName = $extras['installer-name'];
        } elseif (isset($autoload['psr-0']) && is_array($autoload['psr-0'])) {
            $namespace = key($autoload['psr-0']);
            $flowPackageName = str_replace('\\', '.', $namespace);
        } elseif (isset($autoload['psr-4']) && is_array($autoload['psr-4'])) {
            $namespace = key($autoload['psr-4']);
            $flowPackageName = rtrim(str_replace('\\', '.', $namespace), '.');
        } else {
            if (isset($extras['neos']['package-key']) && $this->isPackageKeyValid($extras['neos']['package-key'])) {
                $name = $extras['neos']['package-key'];
            } else {
                $name = $package->getName();
            }
            $nameParts = explode('/', $name);
            $nameParts = array_map(function ($element) {
                $subParts = explode('-', $element);
                $subParts = array_map('ucfirst', $subParts);
                return implode('.', $subParts);
            }, $nameParts);
            $flowPackageName = implode('.', $nameParts);
        }

        return $flowPackageName;
    }

    /**
     * Check the conformance of the given package key
     *
     * @param string $packageKey The package key to validate
     * @return boolean If the package key is valid, returns TRUE otherwise FALSE
     */
    public function isPackageKeyValid($packageKey)
    {
        return preg_match(self::PATTERN_MATCH_PACKAGEKEY, $packageKey) === 1;
    }

}
