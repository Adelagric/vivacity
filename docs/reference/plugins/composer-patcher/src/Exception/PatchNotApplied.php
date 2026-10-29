<?php

namespace ComposerPatcher\Exception;

use ComposerPatcher\Exception;
use ComposerPatcher\Patch;

/**
 * An exception thrown when a patch can't be applied.
 */
class PatchNotApplied extends Exception
{
    /**
     * The path that could not be applied.
     *
     * @var \ComposerPatcher\Patch
     */
    private $patch;

    /**
     * @param Patch $patch the path that could not be applied
     * @param string $reason the description why the patch could not be applied
     */
    public function __construct(Patch $patch, $reason = '')
    {
        $this->patch = $patch;
        $message = 'Unable to apply the patch "'.$patch->getDescription().'" provided by "'.$patch->getFromPackage()->getName().'"';
        $reason = (string) $reason;
        if ($reason === '') {
            $message .= '.';
        } else {
            $message .= ': '.$reason;
        }
        parent::__construct($message);
    }

    /**
     * Get the path that could not be applied.
     *
     * @return \ComposerPatcher\Patch
     */
    public function getPatch()
    {
        return $this->patch;
    }
}
