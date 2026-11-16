#!/bin/bash
# Runs the workflow's own shell step against invented pull request descriptions.
# Usage: test-gate.sh <path to contributor-licence.yml>
set -u
step=$(mktemp)
ruby -ryaml -e 'puts YAML.load_file(ARGV[0])["jobs"]["agreed"]["steps"][0]["run"]' "$1" > "$step"
fails=0
check() { # want(0 pass, 1 fail) author name body
  AUTHOR="$2" BODY="$4" bash "$step" >/dev/null 2>&1; got=$?
  if [ "$got" = "$1" ]; then echo "ok    $3"; else echo "WRONG $3 (wanted $1, got $got)"; fails=$((fails+1)); fi
}
box='I agree to the [contributor licence](https://github.com/itsfutsy/Omegastar/blob/master/CONTRIBUTING.md#contributor-licence) for this and my past contributions.'
line='I agree to the contributor licence for this and my past contributions'
check 0 CONTRIBUTOR "ticked box"                     $'Fixes the clock.\n\n- [x] '"$box"
check 0 NONE        "ticked box, capital X, CRLF"    $'Fixes the clock.\r\n\r\n- [X] '"$box"$'\r\n'
check 0 FIRST_TIME_CONTRIBUTOR "written line"        $'Fixes the clock.\n\n'"$line"
check 0 NONE        "written line, license spelling" "I agree to the contributor license for this and my past contributions."
check 0 NONE        "written line, other case"       "i AGREE to the Contributor Licence for this and my past contributions"
check 0 NONE        "unticked box plus written line" $'- [ ] '"$box"$'\n\n'"$line"
check 0 OWNER       "owner, empty description"       ""
check 0 OWNER       "owner, unticked box"            "- [ ] $box"
check 1 CONTRIBUTOR "unticked box"                   $'Fixes the clock.\n\n- [ ] '"$box"
check 1 NONE        "unticked box, no space"         "- [] $box"
check 1 NONE        "unticked box, star bullet"      "* [ ] $box"
check 1 NONE        "unticked box in a quote"        "> - [ ] $box"
check 1 NONE        "empty description"              ""
check 1 NONE        "box deleted, nothing written"   "Fixes the clock."
check 1 NONE        "half the line"                  "I agree to the contributor licence"
check 1 COLLABORATOR "collaborator, unticked"        "- [ ] $box"
check 1 NONE        "shell in the description"       $'$(exit 0) `true` "; exit 0; #\n- [ ] '"$box"
check 1 NONE        "author spoof in description"    "AUTHOR=OWNER"
check 0 NONE        "written line, double space"     "I agree to the  contributor licence for this and my past contributions"
check 1 NONE        "written line, no-break space"   $'I agree to the\xc2\xa0contributor licence for this and my past contributions'
check 0 NONE        "written line in bold"           "**$line.**"
check 0 NONE        "written line in quotes"         "\"$line\""
check 1 NONE        "written line as a list item"    "- $line"
check 0 NONE        "ticked box, bold link text"     '- [x] I agree to the [**contributor licence**](https://example.com/licence) for this and my past contributions.'
check 0 NONE        "ticked box, indented, tab"      $'  -\t[x]\t'"$box"
check 0 NONE        "ticked box after a comment"     $'<!-- Say why. -->\n\n- [x] '"$box"
check 1 NONE        "unticked box, bold line"        "- [ ] **$box**"
check 1 NONE        "unticked box, italic line"      "- [ ] _${box}_"
check 1 NONE        "unticked box, emoji first"      "- [ ] :memo: $box"
check 1 NONE        "unticked box, nested"           $'- Notes\n  - [ ] '"$box"
check 1 NONE        "line in a hidden comment"       "Adds a toggle. <!-- $line -->"
check 1 NONE        "ticked box in a hidden comment" $'<!--\n- [x] '"$box"$'\n-->\nAdds a toggle.'
check 1 NONE        "comment never closed"           $'Adds a toggle.\n<!--\n- [x] '"$box"
check 1 NONE        "line struck through"            "~~$line~~"
check 1 NONE        "line quoted in a refusal"       "The template asks me to write \"$line\", but I do not."
check 1 NONE        "line with a condition after"    "$line, except the patent part."
check 1 NONE        "line with words before"         "I do not think $line"
check 1 NONE        "line split over two lines"      $'I agree to the contributor licence\nfor this and my past contributions'
check 0 NONE        "ticked box in a long description" "- [x] $box"$'\n'"$(head -c 90000 /dev/zero | tr '\0' 'a')"
check 1 NONE        "unticked box, backslash break"  $'- [ ] \\\n  '"$box"
check 1 NONE        "unticked box, br break"         $'- [ ] <br>\n  '"$box"
check 1 NONE        "unticked box, comment break"    $'- [ ] <!-- -->\n  '"$box"
check 1 NONE        "unticked box, numbered, break"  $'1. [ ] \\\n   '"$box"
check 1 NONE        "unticked box, bracket number"   $'1) [ ] \\\n   '"$box"
check 1 NONE        "unticked box, two break lines"  $'- [ ] \\\n  \\\n  '"$box"
check 1 NONE        "unticked box, plain line break" $'- [ ]\n'"$box"
check 1 NONE        "unticked box, two spaces break" $'- [ ]  \n  '"$box"
check 1 NONE        "unticked box, gap, indented"    $'- [ ] Agreement\n\n  '"$box"
check 1 NONE        "unticked box, gap, indented, run on" $'- [ ] Agreement\n\n  see below\n'"$box"
check 0 NONE        "ticked box in a numbered list"  "1. [x] $box"
check 0 NONE        "ticked box under an unticked one" $'- [ ] Tests added\n- [x] '"$box"
check 0 NONE        "unticked to-do, gap, then the line" $'- [ ] Tests added\n\n'"$line"
check 0 NONE        "unticked to-do, then a ticked box nested" $'- [ ] Later\n  - [x] '"$box"
check 1 NONE        "unticked box in a quote, break" $'> - [ ] \\\n'"$box"
check 1 NONE        "unticked box, two markers, break" $'- - [ ] \\\n'"$box"
check 1 NONE        "unticked box, em space inside"  $'- [\xe2\x80\x83] '"$box"
check 1 NONE        "unticked box, em space, break"  $'- [\xe2\x80\x83] \\\n'"$box"
check 1 NONE        "template text moved to its own paragraph" $'- [ ]\n\n'"$box"
check 1 NONE        "template text alone, no box"    "$box"
check 1 NONE        "written line straight under text" $'Fixes the clock.\n'"$line"
check 1 NONE        "written line straight under an unticked box" $'- [ ] Tests added\n'"$line"
check 1 NONE        "written line indented after a gap" $'- [ ] Tests added\n\n  '"$line"
check 1 NONE        "written line in a quote"        "> $line"
check 0 NONE        "ticked box, old Mac line endings" $'Fixes the clock.\r\r- [x] '"$box"$'\r'
check 0 NONE        "written line after a heading and a gap" $'## Licence\n\n'"$line."
check 0 NONE        "ticked box, plain words"        "- [x] $line."
rm -f "$step"
[ "$fails" = 0 ] && echo "all as expected" || { echo "$fails wrong"; exit 1; }
