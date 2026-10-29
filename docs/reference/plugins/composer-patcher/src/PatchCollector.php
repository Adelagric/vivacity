<?php

namespace ComposerPatcher;

use Composer\Installer\InstallationManager;
use Composer\IO\IOInterface;
use Composer\Json\JsonFile;
use Composer\Package\PackageInterface;
use Composer\Package\RootPackageInterface;
use ComposerPatcher\Util\PathResolver;
use Exception as BaseException;

/**
 * The class that collects patches from packages.
 */
class PatchCollector
{
    /**
     * The PathResoler instance to be used to resolve/download patch files.
     *
     * @var \ComposerPatcher\Util\PathResolver
     */
    private $pathResolver;

    /**
     * The IOInterface instance to be used for user interaction.
     *
     * @var \Composer\IO\IOInterface
     */
    private $io;

    /**
     * The InstallationManager instance to be used to resolve package installation paths.
     *
     * @var \Composer\Installer\InstallationManager
     */
    private $installationManager;

    /**
     * Should errors be considered as warnings (printed to the IO) instead of throwing exceptions?
     *
     * @var bool
     */
    private $errorsAsWarnings;

    /**
     * Initialize the instance.
     *
     * @param \ComposerPatcher\Util\PathResolver $pathResolver the PathResoler instance to be used to resolve/download patch files
     * @param \Composer\IO\IOInterface $io the IOInterface instance to be used for user interaction
     * @param \Composer\Installer\InstallationManager $installationManager the InstallationManager instance to be used to resolve package installation paths
     * @param bool $errorsAsWarnings should errors be considered as warnings (printed to the IO) instead of throwing exceptions?
     */
    public function __construct(PathResolver $pathResolver, IOInterface $io, InstallationManager $installationManager, $errorsAsWarnings)
    {
        $this->pathResolver = $pathResolver;
        $this->io = $io;
        $this->installationManager = $installationManager;
        $this->errorsAsWarnings = $errorsAsWarnings;
    }

    /**
     * Collect the patches.
     *
     * @param \Composer\Package\RootPackageInterface $rootPackage the root composer package
     * @param \Composer\Package\PackageInterface[] $subPackages the dependency packages
     *
     * @return \ComposerPatcher\PatchCollection
     */
    public function collectPatches(RootPackageInterface $rootPackage, array $subPackages)
    {
        $collectedPatches = $this->collectPatchesFromPackage($rootPackage);
        $extra = $rootPackage->getExtra();
        if (isset($extra['allow-subpatches'])) {
            $allowedSubpatches = $extra['allow-subpatches'];
            if (!\is_bool($allowedSubpatches) && !\is_array($allowedSubpatches)) {
                $this->handleException(new Exception\InvalidPackageConfigurationValue($rootPackage, 'extra.allow-subpatches', $allowedSubpatches, 'The extra.allow-subpatches must be a boolean or an array of strings.'));
            }
            $collectedPatches->merge($this->collectSubpackagePatches($subPackages, $allowedSubpatches));
        }

        return $collectedPatches;
    }

    /**
     * Collect the patches from the sub-packages.
     *
     * @param \Composer\Package\PackageInterface[] $subPackages the dependency packages
     * @param string[]|bool $allowedSubpatches TRUE to allow any sub-package; FALSE to allow no sub-packages; an array with allowed package names
     *
     * @return \ComposerPatcher\PatchCollection
     */
    protected function collectSubpackagePatches(array $subPackages, $allowedSubpatches)
    {
        $collectedPatches = new PatchCollection();
        if ($allowedSubpatches !== false && $allowedSubpatches !== array()) {
            foreach ($subPackages as $subPackage) {
                if ($allowedSubpatches === true || \in_array($subPackage->getName(), $allowedSubpatches, true)) {
                    $collectedPatches->merge($this->collectPatchesFromPackage($subPackage));
                }
            }
        }

        return $collectedPatches;
    }

    /**
     * Collect the patches from a package.
     *
     * @param \Composer\Package\PackageInterface $package the package to be inspected
     *
     * @return \ComposerPatcher\PatchCollection
     */
    protected function collectPatchesFromPackage(PackageInterface $package)
    {
        $collectedPatches = new PatchCollection();
        $extra = $package->getExtra();
        if (isset($extra['patches'])) {
            $this->io->write('<info>Gathering patches from '.$package->getName().' (extra.patches).</info>');
            $collectedPatches->merge($this->resolveExplicitList($package, $extra['patches']));
        }
        if (isset($extra['patches-file'])) {
            $this->io->write('<info>Gathering patches from '.$package->getName().' (extra.patches-file).</info>');
            $collectedPatches->merge($this->resolveJsonFile($package, $extra['patches-file']));
        }

        return $collectedPatches;
    }

    /**
     * Collect the patches from the "patches" configuration key.
     *
     * @param \Composer\Package\PackageInterface $package the package being inspected
     * @param array|mixed $patches the extra.patches configuration provided by the package
     *
     * @throws \ComposerPatcher\Exception\InvalidPackageConfigurationValue when $patches is not valid
     *
     * @return \ComposerPatcher\PatchCollection
     */
    protected function resolveExplicitList(PackageInterface $package, $patches)
    {
        $collectedPatches = new PatchCollection();
        if (!\is_array($patches)) {
            $this->handleException(Exception\InvalidPackageConfigurationValue($package, 'extra.patches', $patches, 'The extra.patches configuration must be an array.'));
        } else {
            // If the package is the RootPackage, we won't be able to get its install path from the installationmanager
            if ($package instanceof RootPackageInterface) {
                $packageDirectory = getcwd();
            } else {
                $packageDirectory = $this->installationManager->getInstallPath($package);
            }
            foreach ($patches as $forPackageHandle => $patchList) {
                if (!\is_array($patchList)) {
                    $this->handleException(Exception\InvalidPackageConfigurationValue($package, 'extra.patches', $package, "The \"{$forPackageHandle}\" value must be an array."));
                    continue;
                }
                foreach ($patchList as $patchDescription => $patchData) {
                    try {
                        list($path, $levels) = $this->extractPatchData($package, $patchData);
                        $localFile = $this->pathResolver->resolve($path, $packageDirectory);
                        if ('' === $localFile) {
                            $this->handleException(Exception\InvalidPackageConfigurationValue($package, 'extra.patches', $package, "The path of the \"{$patchDescription}\" patch is empty or is not a string."));
                        }
                        $collectedPatches->addPatch(
                            new Patch($package, $forPackageHandle, $path, $localFile, $patchDescription, $levels)
                        );
                    } catch (BaseException $x) {
                        $this->handleException($x);
                    }
                }
            }
        }

        return $collectedPatches;
    }

    /**
     * Extract the data of a single patch.
     *
     * @param \Composer\Package\PackageInterface $package the package being inspected
     * @param array|string|mixed $patchData The single patch data
     *
     * @throws \Exception in case of errors
     *
     * @return array
     */
    protected function extractPatchData(PackageInterface $package, $patchData)
    {
        $levels = null;
        if (\is_string($patchData)) {
            $path = $patchData;
        } else {
            if (!\is_array($patchData) || !isset($patchData['path']) || !\is_string($patchData['path'])) {
                throw new Exception\InvalidPackageConfigurationValue($package, 'extra.patches.[...]', $package, "The value of a patch must be a string or an array with a 'path' node value.");
            }
            $path = $patchData['path'];
            if (isset($patchData['levels'])) {
                $levels = $patchData['levels'];
                if (!\is_array($levels) || $levels === array()) {
                    throw new Exception\InvalidPackageConfigurationValue($package, 'extra.patches.[...].levels', $package, 'The patch levels must be an array of strings.');
                }
            }
        }
        if (null === $levels) {
            $levels = $this->getDefaultPatchLevels();
        }

        return array($path, $levels);
    }

    /**
     * Collect the patches from the "patches-file" configuration key.
     *
     * @param \Composer\Package\PackageInterface $package the package being inspected
     * @param string|mixed $jsonPath the extra.patches-file configuration provided by the package
     *
     * @throws \ComposerPatcher\Exception\InvalidPackageConfigurationValue when $jsonPath is not valid
     *
     * @return \ComposerPatcher\PatchCollection
     */
    protected function resolveJsonFile(PackageInterface $package, $jsonPath)
    {
        $collectedPatches = new PatchCollection();
        if (!\is_string($jsonPath) || '' === $jsonPath) {
            $this->handleException(Exception\InvalidPackageConfigurationValue($package(), 'extra.patches-file', $jsonPath, 'The extra.patches-file configuration must be a non empty string.'));
        } else {
            $packageDirectory = $this->installationManager->getInstallPath($package);
            $fullJsonPath = $this->pathResolver->resolve($jsonPath, $packageDirectory);
            $jsonReader = new JsonFile($fullJsonPath, null, $this->io);
            $data = $jsonReader->read();
            if (!\is_array($data)) {
                $this->handleException(Exception\InvalidPackageConfigurationValue($package, 'extra.patches-file', $jsonPath, "The JSON file at \"{$jsonPath}\" must contain an array."));
            } elseif (!isset($data['patches'])) {
                $this->handleException(Exception\InvalidPackageConfigurationValue($package, 'extra.patches-file', $jsonPath, "The JSON file at \"{$jsonPath}\" must contain an array with a \"patches\" key."));
            } else {
                $collectedPatches->merge($this->resolveExplicitList($package, $data['patches']));
            }
        }

        return $collectedPatches;
    }

    /**
     * Throws or prints out an error accordingly to $errorsAsWarnings.
     *
     * @param \Exception $exception The Exception to be reported
     *
     * @throws \Exception throws $exception if $errorsAsWarnings is false, prints it out if $errorsAsWarnings is true
     */
    protected function handleException(BaseException $exception)
    {
        if (!$this->errorsAsWarnings) {
            throw $exception;
        }
        $this->io->write('<error>'.$exception->getMessage().'</error>');
    }

    /**
     * Get the default patch levels.
     *
     * @return string[]
     */
    protected function getDefaultPatchLevels()
    {
        return array('-p1', '-p0', '-p2', '-p4');
    }
}
