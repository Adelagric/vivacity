<?php

namespace ComposerPatcher\Exception;

use Composer\Package\PackageInterface;
use ComposerPatcher\Exception;

/**
 * An exception thrown when a package contains an invalid configuration value.
 */
class InvalidPackageConfigurationValue extends Exception
{
    /**
     * The package that contains the invalid configuration value.
     *
     * @var \Composer\Package\PackageInterface
     */
    protected $package;

    /**
     * The name of the invalid configuration value.
     *
     * @var string
     */
    protected $configurationName;

    /**
     * The value of the invalid configuration.
     */
    protected $configurationValue;

    /**
     * The reason why the message value is not valid.
     *
     * @var string
     */
    protected $invalidValueDescription;

    /**
     * @param string $configurationName the name of the invalid configuration value
     * @param mixed $configurationValue the value of the invalid configuration
     * @param string $invalidValueDescription the reason why the message value is not valid
     */
    public function __construct(PackageInterface $package, $configurationName, $configurationValue, $invalidValueDescription)
    {
        $this->package = $package;
        $this->configurationName = $configurationName;
        $this->configurationValue = $configurationValue;
        $this->invalidValueDescription = $invalidValueDescription;
        $message = 'The package "'.$package->getName().'" contains an invalid value of the "'.$configurationValue.'" configuration key: '.$invalidValueDescription;
        parent::__construct($message);
    }

    /**
     * Get the package that contains the invalid configuration value.
     *
     * @return \Composer\Package\PackageInterface
     */
    public function getPackage()
    {
        return $this->package;
    }

    /**
     * Get the name of the invalid configuration value.
     *
     * @return string
     */
    public function getConfigurationName()
    {
        return $this->configurationName;
    }

    /**
     * Get the value of the invalid configuration.
     */
    public function getConfigurationValue()
    {
        return $this->configurationValue;
    }

    /**
     * Get the reason why the message value is not valid.
     *
     * @return string
     */
    public function getInvalidValueDescription()
    {
        return $this->invalidValueDescription;
    }
}
