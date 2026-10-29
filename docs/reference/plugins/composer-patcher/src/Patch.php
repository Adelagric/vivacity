<?php

namespace ComposerPatcher;

use Composer\Package\PackageInterface;
use Composer\Semver\Constraint\Constraint;
use Composer\Semver\VersionParser;

/**
 * Represent a single patch provided by a package.
 */
class Patch
{
    /**
     * The package that provides the patch.
     *
     * @var \Composer\Package\PackageInterface
     */
    private $fromPackage;

    /**
     * The name of the package the patch is for.
     *
     * @var string
     */
    private $forPackage;

    /**
     * The package version constraint.
     *
     * @var \Composer\Semver\Constraint\ConstraintInterface|null
     */
    private $forPackageVersion;

    /**
     * The path the patch as specified by the package.
     *
     * @var string
     */
    private $originalPath;

    /**
     * The absolute path of the local patch file.
     *
     * @var string
     */
    private $localPath;

    /**
     * The description of the patch.
     *
     * @var string
     */
    private $description;

    /**
     * The patch levels.
     *
     * @var string
     */
    private $levels;

    /**
     * The patch hash.
     *
     * @var string|null
     */
    private $hash;

    /**
     * @param \Composer\Package\PackageInterface $fromPackage the package that provides the patch
     * @param string $forPackage the name of the package the patch is for (may include versions after a colon)
     * @param string $originalPath the path the patch as specified by the package
     * @param string $localPath the absolute path of the local patch file
     * @param string $description the description of the patch
     * @param string[] $levels the patch levels
     */
    public function __construct(PackageInterface $fromPackage, $forPackage, $originalPath, $localPath, $description, array $levels)
    {
        $this->fromPackage = $fromPackage;
        list($this->forPackage, $forPackageVersionString) = $this->parsePackageVersion($forPackage);
        if ($forPackageVersionString === '*') {
            $this->forPackageVersion = null;
        } else {
            $versionParser = new VersionParser();
            $this->forPackageVersion = $versionParser->parseConstraints($forPackageVersionString);
        }

        $this->originalPath = $originalPath;
        $this->localPath = $localPath;
        $this->description = $description;
        $this->levels = $levels;
    }

    /**
     * Ge the package that provides the patch.
     *
     * @return \Composer\Package\PackageInterface
     */
    public function getFromPackage()
    {
        return $this->fromPackage;
    }

    /**
     * Get the name of the package the patch is for.
     *
     * @return string
     */
    public function getForPackage()
    {
        return $this->forPackage;
    }

    /**
     * Get the version constraint of the package the patch is for (if any).
     *
     * @return \Composer\Semver\Constraint\ConstraintInterface|null
     */
    public function getForPackageVersion()
    {
        return $this->forPackageVersion;
    }

    /**
     * Get the path the patch as specified by the package.
     *
     * @return string
     */
    public function getOriginalPath()
    {
        return $this->originalPath;
    }

    /**
     * Get the absolute path of the local patch file.
     *
     * @return string
     */
    public function getLocalPath()
    {
        return $this->localPath;
    }

    /**
     * Get the description of the patch.
     *
     * @return string
     */
    public function getDescription()
    {
        return $this->description;
    }

    /**
     * Get the patch levels.
     *
     * @return string[]
     */
    public function getLevels()
    {
        return $this->levels;
    }

    /**
     * Check if this patch is for a package.
     *
     * @return bool
     */
    public function isForPackage(PackageInterface $package)
    {
        if ($this->getForPackage() !== $package->getName()) {
            return false;
        }
        $fpv = $this->getForPackageVersion();
        if ($fpv !== null) {
            $pv = new Constraint('=', $package->getVersion());
            if (!$fpv->matches($pv)) {
                return false;
            }
        }

        return true;
    }

    /**
     * Get the hash of this patch.
     *
     * @return string
     */
    public function getHash()
    {
        if ($this->hash === null) {
            $this->hash = sha1_file($this->getLocalPath());
        }

        return $this->hash;
    }

    /**
     * @param string $package
     *
     * @return string[]
     */
    protected function parsePackageVersion($package)
    {
        $parts = explode(':', $package, 2);
        $parts[1] = isset($parts[1]) ? trim($parts[1]) : '*';
        if ($parts[1] !== '') {
            $name = $parts[0];
            $version = $parts[1];
        } else {
            $name = $package;
            $version = '*';
        }

        return array($name, $version);
    }
}
