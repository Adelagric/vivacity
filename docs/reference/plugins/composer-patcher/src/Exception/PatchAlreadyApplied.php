<?php

namespace ComposerPatcher\Exception;

use ComposerPatcher\Exception;
use ComposerPatcher\Patch;

/**
 * An exception thrown while trying to apply an already applied patch.
 */
class PatchAlreadyApplied extends Exception
{
    /**
     * The already applied patch.
     *
     * @var \ComposerPatcher\Patch
     */
    protected $patch;

    /**
     * @param \ComposerPatcher\Patch $patch the already applied patch
     * @param string $message an optional message
     */
    public function __construct(Patch $patch, $message = '')
    {
        $this->patch = $patch;
        $message = (string) $message;
        if ($message === '') {
            $packageVersion = $patch->getForPackageVersion();
            $message = 'The patch "'.$patch->getDescription().'"';
            if ($packageVersion !== null) {
                $message .= ' for '.$packageVersion->getPrettyString();
            }
            $message .= ' provided by '.$patch->getFromPackage()->__toString().' is already applied.';
        }
        parent::__construct($message);
    }

    /**
     * Get the already applied patch.
     *
     * @return \ComposerPatcher\Patch
     */
    public function getPatch()
    {
        return $this->patch;
    }
}
