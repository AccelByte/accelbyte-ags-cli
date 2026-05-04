# AGS CLI Command Catalogue

Auto-generated from `specs/*.json.gz` via `scripts/generate_cli_command_catalogue.py`. Do not hand-edit.

Lists every operation the CLI dispatches. Deprecated operations and the `internal` resource are excluded. Each row corresponds to a concrete `--api-scope` / `--api-version` combination.

**Services:** 24
**Operations:** 1961

## achievement

- Spec name: `achievement`
- Resources: 5
- Operations: 28

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `achievements` | `create` | admin | v1 | POST | `/achievement/v1/admin/namespaces/{namespace}/achievements` | Creates a new achievement |
| `achievements` | `delete` | admin | v1 | DELETE | `/achievement/v1/admin/namespaces/{namespace}/achievements/{achievementCode}` | Deletes an achievement |
| `achievements` | `export` | admin | v1 | GET | `/achievement/v1/admin/namespaces/{namespace}/achievements/export` | Exports achievement configuration |
| `achievements` | `get` | admin | v1 | GET | `/achievement/v1/admin/namespaces/{namespace}/achievements/{achievementCode}` | Retrieves an achievement |
| `achievements` | `get` | public | v1 | GET | `/achievement/v1/public/namespaces/{namespace}/achievements/{achievementCode}` | Retrieves an achievement |
| `achievements` | `import` | admin | v1 | POST | `/achievement/v1/admin/namespaces/{namespace}/achievements/import` | Imports achievement configuration |
| `achievements` | `list` | admin | v1 | GET | `/achievement/v1/admin/namespaces/{namespace}/achievements` | Lists achievements |
| `achievements` | `list` | public | v1 | GET | `/achievement/v1/public/namespaces/{namespace}/achievements` | Lists achievements |
| `achievements` | `reorder` | admin | v1 | PATCH | `/achievement/v1/admin/namespaces/{namespace}/achievements/{achievementCode}` | Updates achievement list order |
| `achievements` | `update` | admin | v1 | PUT | `/achievement/v1/admin/namespaces/{namespace}/achievements/{achievementCode}` | Updates an achievement |
| `collaborative-progress` | `claim-reward` | public | v1 | POST | `/achievement/v1/public/namespaces/{namespace}/users/{userId}/global/achievements/{achievementCode}/claim` | Claims a global achievement reward |
| `collaborative-progress` | `list` | admin | v1 | GET | `/achievement/v1/admin/namespaces/{namespace}/global/achievements` | Lists global achievements |
| `collaborative-progress` | `list` | public | v1 | GET | `/achievement/v1/public/namespaces/{namespace}/global/achievements` | Lists global achievements |
| `collaborative-progress` | `list-contributions` | admin | v1 | GET | `/achievement/v1/admin/namespaces/{namespace}/users/{userId}/global/achievements` | Lists user global achievement contributions |
| `collaborative-progress` | `list-contributions` | public | v1 | GET | `/achievement/v1/public/namespaces/{namespace}/users/{userId}/global/achievements` | Lists user global achievement contributions |
| `collaborative-progress` | `list-contributors` | admin | v1 | GET | `/achievement/v1/admin/namespaces/{namespace}/global/achievements/{achievementCode}/contributors` | Lists global achievement contributors |
| `collaborative-progress` | `list-contributors` | public | v1 | GET | `/achievement/v1/public/namespaces/{namespace}/global/achievements/{achievementCode}/contributors` | Lists global achievement contributors |
| `collaborative-progress` | `reset` | admin | v1 | DELETE | `/achievement/v1/admin/namespaces/{namespace}/global/achievements/{achievementCode}/reset` | Resets a global achievement |
| `platform-achievements` | `bulk-create-playstation-event` | admin | v1 | POST | `/achievement/v1/admin/namespaces/{namespace}/platforms/psn/bulk` | Creates PSN UDS events |
| `tags` | `list` | admin | v1 | GET | `/achievement/v1/admin/namespaces/{namespace}/tags` | Lists tags |
| `tags` | `list` | public | v1 | GET | `/achievement/v1/public/namespaces/{namespace}/tags` | Lists tags |
| `user-progress` | `bulk-unlock` | admin | v1 | PUT | `/achievement/v1/admin/namespaces/{namespace}/users/{userId}/achievements/bulkUnlock` | Unlocks achievements in bulk |
| `user-progress` | `bulk-unlock` | public | v1 | PUT | `/achievement/v1/public/namespaces/{namespace}/users/{userId}/achievements/bulkUnlock` | Unlocks achievements in bulk |
| `user-progress` | `list` | admin | v1 | GET | `/achievement/v1/admin/namespaces/{namespace}/users/{userId}/achievements` | Lists user achievements |
| `user-progress` | `list` | public | v1 | GET | `/achievement/v1/public/namespaces/{namespace}/users/{userId}/achievements` | Lists user achievements |
| `user-progress` | `reset` | admin | v1 | DELETE | `/achievement/v1/admin/namespaces/{namespace}/users/{userId}/achievements/{achievementCode}/reset` | Resets an achievement |
| `user-progress` | `unlock` | admin | v1 | PUT | `/achievement/v1/admin/namespaces/{namespace}/users/{userId}/achievements/{achievementCode}/unlock` | Unlocks an achievement |
| `user-progress` | `unlock` | public | v1 | PUT | `/achievement/v1/public/namespaces/{namespace}/users/{userId}/achievements/{achievementCode}/unlock` | Unlocks an achievement |

## ams

- Spec name: `ams`
- Resources: 10
- Operations: 45

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `account` | `create` | admin | v1 | POST | `/ams/v1/admin/namespaces/{namespace}/account` | Creates a new AMS account |
| `account` | `get` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/account` | Retrieves the account associated with the namespace |
| `account` | `get` | public | v1 | GET | `/ams/v1/namespaces/{namespace}/account` | Retrieves the account associated with the namespace |
| `account` | `get-link-token` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/account/link` | Retrieves an account link token |
| `account` | `link` | admin | v1 | POST | `/ams/v1/admin/namespaces/{namespace}/account/link` | Links an account to a namespace |
| `artifacts` | `bulk-delete` | admin | v1 | DELETE | `/ams/v1/admin/namespaces/{namespace}/artifacts` | Deletes artifacts in bulk that match any of the provided criteria |
| `artifacts` | `delete` | admin | v1 | DELETE | `/ams/v1/admin/namespaces/{namespace}/artifacts/{artifactID}` | Deletes a specified artifact |
| `artifacts` | `get` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/artifacts` | Retrieves all artifacts that match the provided criteria |
| `artifacts` | `get-download-url` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/artifacts/{artifactID}/url` | Retrieves a signed URL for a specific artifact |
| `artifacts` | `get-fleet-sampling-rules` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/fleets/{fleetID}/artifacts-sampling-rules` | Returns the sampling rules for a fleet |
| `artifacts` | `get-usage` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/artifacts/usage` | Retrieves artifact storage usage for the namespace |
| `artifacts` | `set-fleet-sampling-rules` | admin | v1 | PUT | `/ams/v1/admin/namespaces/{namespace}/fleets/{fleetID}/artifacts-sampling-rules` | Sets sampling rules for a fleet |
| `dedicated-servers` | `claim-by-fleet-id` | public | v1 | PUT | `/ams/v1/namespaces/{namespace}/fleets/{fleetID}/claim` | Claims a dedicated server from a fleet |
| `dedicated-servers` | `claim-by-keys` | public | v1 | PUT | `/ams/v1/namespaces/{namespace}/servers/claim` | Claims a dedicated server |
| `dedicated-servers` | `get-connection-info` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/servers/{serverID}/connectioninfo` | Retrieves dedicated server connection info |
| `dedicated-servers` | `get-history` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/servers/{serverID}/history` | Retrieves dedicated server history |
| `dedicated-servers` | `get-history-by-fleet-id` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/fleets/{fleetID}/servers/history` | Retrieves history records for a dedicated server in a fleet |
| `dedicated-servers` | `get-info` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/servers/{serverID}` | Retrieves dedicated server details |
| `dev-server-config` | `create` | admin | v1 | POST | `/ams/v1/admin/namespaces/{namespace}/development/server-configurations` | Creates a new development server configuration |
| `dev-server-config` | `delete` | admin | v1 | DELETE | `/ams/v1/admin/namespaces/{namespace}/development/server-configurations/{developmentServerConfigID}` | Deletes a development server configuration |
| `dev-server-config` | `get` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/development/server-configurations/{developmentServerConfigID}` | Retrieves a development server configuration |
| `dev-server-config` | `list` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/development/server-configurations` | Lists development server configurations with pagination |
| `dev-server-config` | `update` | admin | v1 | PATCH | `/ams/v1/admin/namespaces/{namespace}/development/server-configurations/{developmentServerConfigID}` | Updates a development server configuration |
| `fleet-commander` | `check-auth` | public | v1 | GET | `/ams/auth` | Checks if fleet commander can auth with AMS |
| `fleet-commander` | `get-version` | public | v1 | GET | `/ams/version` | Retrieves version info |
| `fleets` | `bulk-delete` | admin | v1 | DELETE | `/ams/v1/admin/namespaces/{namespace}/fleets` | Deletes one or more fleets |
| `fleets` | `create` | admin | v1 | POST | `/ams/v1/admin/namespaces/{namespace}/fleets` | Creates a fleet |
| `fleets` | `delete` | admin | v1 | DELETE | `/ams/v1/admin/namespaces/{namespace}/fleets/{fleetID}` | Deletes a fleet |
| `fleets` | `get` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/fleets/{fleetID}` | Retrieves a fleet |
| `fleets` | `get-dedicated-server` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/fleets/{fleetID}/servers` | Retrieves server details and counts for a fleet |
| `fleets` | `list` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/fleets` | Lists all fleets in a namespace |
| `fleets` | `update` | admin | v1 | PUT | `/ams/v1/admin/namespaces/{namespace}/fleets/{fleetID}` | Updates a fleet |
| `images` | `get` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/images/{imageID}` | Retrieves image details |
| `images` | `get-storage` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/images-storage` | Retrieves current usage for image storage |
| `images` | `list` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/images` | Retrieves a list of existing images |
| `images` | `mark-for-deletion` | admin | v1 | DELETE | `/ams/v1/admin/namespaces/{namespace}/images/{imageID}` | Marks the image for deletion |
| `images` | `unmark-for-deletion` | admin | v1 | POST | `/ams/v1/admin/namespaces/{namespace}/images/{imageID}/restore` | Returns an image to active use |
| `images` | `update` | admin | v1 | PATCH | `/ams/v1/admin/namespaces/{namespace}/images/{imageID}` | Updates an image |
| `info` | `get-upload-url` | public | v1 | GET | `/ams/v1/upload-url` | Retrieves an upload URL for an image |
| `info` | `list-regions` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/regions` | Retrieves available AMS regions |
| `info` | `list-supported-insstances` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/supported-instances` | Retrieves available instance types for the current account |
| `qos` | `list-regions` | admin | v1 | GET | `/ams/v1/admin/namespaces/{namespace}/qos` | Retrieves available AMS QoS regions |
| `qos` | `update-region` | admin | v1 | PATCH | `/ams/v1/admin/namespaces/{namespace}/qos/{region}` | Updates a QoS region status |
| `watchdogs` | `connect` | public | v1 | GET | `/ams/v1/namespaces/{namespace}/watchdogs/{watchdogID}/connect` | Connects a watchdog |
| `watchdogs` | `connect-local` | public | v1 | GET | `/ams/v1/namespaces/{namespace}/local/{watchdogID}/connect` | Connects a local watchdog |

## basic

- Spec name: `basic`
- Resources: 5
- Operations: 57

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `config` | `create` | admin | v1 | POST | `/basic/v1/admin/namespaces/{namespace}/configs` | Creates a config |
| `config` | `delete` | admin | v1 | DELETE | `/basic/v1/admin/namespaces/{namespace}/configs/{configKey}` | Deletes a config |
| `config` | `get` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/configs/{configKey}` | Retrieves a config |
| `config` | `get-publisher` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/publisher/configs/{configKey}` | Retrieves a publisher config |
| `config` | `update` | admin | v1 | PATCH | `/basic/v1/admin/namespaces/{namespace}/configs/{configKey}` | Updates a config |
| `files` | `generate-upload-url` | admin | v1 | POST | `/basic/v1/admin/namespaces/{namespace}/folders/{folder}/files` | Generates an upload URL |
| `files` | `generate-upload-url` | public | v1 | POST | `/basic/v1/public/namespaces/{namespace}/folders/{folder}/files` | Generates an upload URL |
| `files` | `generate-user-upload-url` | admin | v1 | POST | `/basic/v1/admin/namespaces/{namespace}/users/{userId}/files` | Generates an upload URL for user content |
| `files` | `generate-user-upload-url` | public | v1 | POST | `/basic/v1/public/namespaces/{namespace}/users/{userId}/files` | Generates an upload URL for user content |
| `locale` | `add-country-group` | admin | v1 | POST | `/basic/v1/admin/namespaces/{namespace}/misc/countrygroups` | Adds a country group |
| `locale` | `delete-country-group` | admin | v1 | DELETE | `/basic/v1/admin/namespaces/{namespace}/misc/countrygroups/{countryGroupCode}` | Deletes a country group |
| `locale` | `get-time` | public | v1 | GET | `/basic/v1/public/misc/time` | Retrieves server time |
| `locale` | `list-country-groups` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/misc/countrygroups` | Lists country groups |
| `locale` | `list-languages` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/misc/languages` | Lists languages |
| `locale` | `list-languages` | public | v1 | GET | `/basic/v1/public/namespaces/{namespace}/misc/languages` | Lists languages |
| `locale` | `list-time-zones` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/misc/timezones` | Lists time zones |
| `locale` | `list-time-zones` | public | v1 | GET | `/basic/v1/public/namespaces/{namespace}/misc/timezones` | Lists time zones |
| `locale` | `update-country-group` | admin | v1 | PUT | `/basic/v1/admin/namespaces/{namespace}/misc/countrygroups/{countryGroupCode}` | Updates a country group |
| `namespaces` | `create` | admin | v1 | POST | `/basic/v1/admin/namespaces` | Creates a namespace |
| `namespaces` | `delete` | admin | v1 | DELETE | `/basic/v1/admin/namespaces/{namespace}` | Deletes a namespace |
| `namespaces` | `get` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}` | Retrieves a namespace |
| `namespaces` | `get` | public | v1 | GET | `/basic/v1/public/namespaces/{namespace}` | Retrieves namespace info |
| `namespaces` | `get-context` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/context` | Retrieves context of namespace |
| `namespaces` | `list` | admin | v1 | GET | `/basic/v1/admin/namespaces` | Retrieves all namespaces |
| `namespaces` | `list` | public | v1 | GET | `/basic/v1/public/namespaces` | Retrieves all namespaces |
| `namespaces` | `list-children` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/child` | Retrieves child namespaces |
| `namespaces` | `list-games` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/game` | Retrieves game namespaces |
| `namespaces` | `list-publisher` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/publisher` | Retrieves publisher namespace info for a namespace |
| `namespaces` | `list-publisher` | public | v1 | GET | `/basic/v1/public/namespaces/{namespace}/publisher` | Retrieves publisher namespace info for a namespace |
| `namespaces` | `update` | admin | v1 | PATCH | `/basic/v1/admin/namespaces/{namespace}/basic` | Updates namespace basic info |
| `namespaces` | `update-status` | admin | v1 | PATCH | `/basic/v1/admin/namespaces/{namespace}/status` | Updates the active status of a namespace |
| `profiles` | `bulk-get-public` | admin | v1 | POST | `/basic/v1/admin/namespaces/{namespace}/profiles/public` | Retrieves public user profile info by user IDs |
| `profiles` | `bulk-get-public` | public | v1 | POST | `/basic/v1/public/namespaces/{namespace}/profiles/public` | Retrieves public user profile info in bulk by user IDs |
| `profiles` | `bulk-get-public-by-user-ids` | public | v1 | GET | `/basic/v1/public/namespaces/{namespace}/profiles/public` | Retrieves public user profile info by user IDs |
| `profiles` | `create` | public | v1 | POST | `/basic/v1/public/namespaces/{namespace}/users/{userId}/profiles` | Creates user profile |
| `profiles` | `create-my` | public | v1 | POST | `/basic/v1/public/namespaces/{namespace}/users/me/profiles` | Creates my profile |
| `profiles` | `delete` | admin | v1 | DELETE | `/basic/v1/admin/namespaces/{namespace}/users/{userId}/profiles` | Deletes user profile |
| `profiles` | `get` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/users/{userId}/profiles` | Retrieves user profile |
| `profiles` | `get` | public | v1 | GET | `/basic/v1/public/namespaces/{namespace}/users/{userId}/profiles` | Retrieves user profile |
| `profiles` | `get-by-public-id` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/profiles/byPublicId` | Retrieves user profile by public ID |
| `profiles` | `get-by-public-id` | public | v1 | GET | `/basic/v1/public/namespaces/{namespace}/profiles/public/byPublicId` | Retrieves public user profile by public ID |
| `profiles` | `get-custom-attributes` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/users/{userId}/profiles/customAttributes` | Retrieves user custom attributes |
| `profiles` | `get-custom-attributes` | public | v1 | GET | `/basic/v1/public/namespaces/{namespace}/users/{userId}/profiles/customAttributes` | Retrieves user custom attributes |
| `profiles` | `get-my` | public | v1 | GET | `/basic/v1/public/namespaces/{namespace}/users/me/profiles` | Retrieves my profile |
| `profiles` | `get-my-private-custom-attributes` | public | v1 | GET | `/basic/v1/public/namespaces/{namespace}/users/me/profiles/privateCustomAttributes` | Retrieves my private custom attributes |
| `profiles` | `get-my-zip-code` | public | v1 | GET | `/basic/v1/public/namespaces/{namespace}/users/me/profiles/zipCode` | Retrieves my zip code |
| `profiles` | `get-private-custom-attributes` | admin | v1 | GET | `/basic/v1/admin/namespaces/{namespace}/users/{userId}/profiles/privateCustomAttributes` | Retrieves user private custom attributes |
| `profiles` | `get-public` | public | v1 | GET | `/basic/v1/public/namespaces/{namespace}/users/{userId}/profiles/public` | Retrieves public user profile info |
| `profiles` | `update` | admin | v1 | PUT | `/basic/v1/admin/namespaces/{namespace}/users/{userId}/profiles` | Updates user profile |
| `profiles` | `update` | public | v1 | PUT | `/basic/v1/public/namespaces/{namespace}/users/{userId}/profiles` | Updates user profile |
| `profiles` | `update-custom-attributes` | admin | v1 | PUT | `/basic/v1/admin/namespaces/{namespace}/users/{userId}/profiles/customAttributes` | Updates custom attributes for a user |
| `profiles` | `update-custom-attributes` | public | v1 | PUT | `/basic/v1/public/namespaces/{namespace}/users/{userId}/profiles/customAttributes` | Updates custom attributes for a user |
| `profiles` | `update-my` | public | v1 | PUT | `/basic/v1/public/namespaces/{namespace}/users/me/profiles` | Updates my profile |
| `profiles` | `update-my-private-custom-attributes` | public | v1 | PUT | `/basic/v1/public/namespaces/{namespace}/users/me/profiles/privateCustomAttributes` | Updates my private custom attributes |
| `profiles` | `update-private-custom-attributes` | admin | v1 | PUT | `/basic/v1/admin/namespaces/{namespace}/users/{userId}/profiles/privateCustomAttributes` | Updates private custom attributes for a user |
| `profiles` | `update-status` | admin | v1 | PATCH | `/basic/v1/admin/namespaces/{namespace}/users/{userId}/profiles/status` | Updates user profile status |
| `profiles` | `update-status` | public | v1 | PATCH | `/basic/v1/public/namespaces/{namespace}/users/{userId}/profiles/status` | Updates user profile status |

## challenge

- Spec name: `challenge`
- Resources: 6
- Operations: 38

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `assignment-plugins` | `create` | admin | v1 | POST | `/challenge/v1/admin/namespaces/{namespace}/plugins/assignment` | Creates an assignment plugin |
| `assignment-plugins` | `delete` | admin | v1 | DELETE | `/challenge/v1/admin/namespaces/{namespace}/plugins/assignment` | Deletes the assignment plugin |
| `assignment-plugins` | `get` | admin | v1 | GET | `/challenge/v1/admin/namespaces/{namespace}/plugins/assignment` | Retrieves the assignment plugin |
| `assignment-plugins` | `update` | admin | v1 | PUT | `/challenge/v1/admin/namespaces/{namespace}/plugins/assignment` | Updates the assignment plugin |
| `challenges` | `create` | admin | v1 | POST | `/challenge/v1/admin/namespaces/{namespace}/challenges` | Creates a new challenge |
| `challenges` | `delete` | admin | v1 | DELETE | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}` | Deletes a challenge |
| `challenges` | `delete-tied` | admin | v1 | DELETE | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}/tied` | Deletes a tied challenge and all related data |
| `challenges` | `get` | admin | v1 | GET | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}` | Retrieves a challenge by code |
| `challenges` | `list` | admin | v1 | GET | `/challenge/v1/admin/namespaces/{namespace}/challenges` | Lists challenges in the namespace |
| `challenges` | `list` | public | v1 | GET | `/challenge/v1/public/namespaces/{namespace}/challenges` | Lists challenges in the namespace |
| `challenges` | `list-active` | admin | v1 | GET | `/challenge/v1/admin/namespaces/{namespace}/challenges/users/{userId}` | Lists a user's active challenges |
| `challenges` | `list-periods` | admin | v1 | GET | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}/periods` | Lists rotation periods of a challenge |
| `challenges` | `randomize` | admin | v1 | POST | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}/randomize` | Triggers goal randomization for a challenge |
| `challenges` | `update` | admin | v1 | PUT | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}` | Updates a challenge |
| `challenges` | `update-tied-schedule` | admin | v1 | PUT | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}/tied/schedule` | Updates the schedule of a tied challenge |
| `goals` | `create` | admin | v1 | POST | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}/goals` | Creates a new goal |
| `goals` | `delete` | admin | v1 | DELETE | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}/goals/{code}` | Deletes a goal |
| `goals` | `get` | admin | v1 | GET | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}/goals/{code}` | Retrieves a goal |
| `goals` | `list` | admin | v1 | GET | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}/goals` | Lists goals of a challenge |
| `goals` | `list` | public | v1 | GET | `/challenge/v1/public/namespaces/{namespace}/challenges/{challengeCode}/goals` | Lists goals of a challenge |
| `goals` | `update` | admin | v1 | PUT | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}/goals/{code}` | Updates a goal |
| `progress` | `claim-my-rewards` | public | v1 | POST | `/challenge/v1/public/namespaces/{namespace}/users/me/rewards/claim` | Claims the current user's rewards |
| `progress` | `claim-my-rewards-by-challenge-code` | public | v1 | POST | `/challenge/v1/public/namespaces/{namespace}/users/me/challenges/{challengeCode}/rewards/claim` | Claims the current user's rewards by goal code |
| `progress` | `claim-rewards` | admin | v1 | POST | `/challenge/v1/admin/namespaces/{namespace}/users/{userId}/rewards/claim` | Claims rewards for a user |
| `progress` | `claim-rewards-by-challenge-code` | admin | v1 | POST | `/challenge/v1/admin/namespaces/{namespace}/users/{userId}/challenges/{challengeCode}/rewards/claim` | Claims rewards for a user by goal code |
| `progress` | `claim-rewards-by-user-ids` | admin | v1 | POST | `/challenge/v1/admin/namespaces/{namespace}/users/rewards/claim` | Claims rewards for a group of users |
| `progress` | `evaluate` | admin | v1 | POST | `/challenge/v1/admin/namespaces/{namespace}/progress/evaluate` | Triggers progression evaluation for users |
| `progress` | `evaluate` | public | v1 | POST | `/challenge/v1/public/namespaces/{namespace}/users/me/progress/evaluate` | Triggers progression evaluation for the current user |
| `progress` | `list` | admin | v1 | GET | `/challenge/v1/admin/namespaces/{namespace}/users/{userId}/progress/{challengeCode}` | Lists a user's goal progressions |
| `progress` | `list-my` | public | v1 | GET | `/challenge/v1/public/namespaces/{namespace}/users/me/progress/{challengeCode}` | Lists the current user's goal progressions |
| `progress` | `list-my-rewards` | public | v1 | GET | `/challenge/v1/public/namespaces/{namespace}/users/me/rewards` | Lists the current user's earned rewards |
| `progress` | `list-past` | public | v1 | GET | `/challenge/v1/public/namespaces/{namespace}/users/me/progress/{challengeCode}/index/{index}` | Lists the current user's progressions in a previous rotation |
| `progress` | `list-user-rewards` | admin | v1 | GET | `/challenge/v1/admin/namespaces/{namespace}/users/{userId}/rewards` | Lists a user's earned rewards |
| `schedules` | `list` | admin | v1 | GET | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}/schedules` | Lists schedules for a challenge |
| `schedules` | `list` | public | v1 | GET | `/challenge/v1/public/namespaces/{namespace}/challenges/{challengeCode}/schedules` | Lists schedules for a challenge |
| `schedules` | `list-by-goal` | admin | v1 | GET | `/challenge/v1/admin/namespaces/{namespace}/challenges/{challengeCode}/goals/{code}/schedules` | Lists schedules for a goal |
| `schedules` | `list-by-goal` | public | v1 | GET | `/challenge/v1/public/namespaces/{namespace}/challenges/{challengeCode}/goals/{code}/schedules` | Lists schedules for a goal |
| `utilities` | `list-item-references` | admin | v1 | GET | `/challenge/v1/admin/namespaces/{namespace}/challenges/item/references` | Retrieves challenge references for an ecommerce item |

## chat

- Spec name: `chat`
- Resources: 6
- Operations: 63

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `config` | `export` | admin | v1 | GET | `/chat/v1/admin/config/namespaces/{namespace}/export` | Exports the chat configuration to a JSON file |
| `config` | `get` | admin | v1 | GET | `/chat/v1/admin/config/namespaces/{namespace}` | Retrieves the chat configuration for a namespace |
| `config` | `get` | public | v1 | GET | `/chat/v1/public/config/namespaces/{namespace}` | Retrieves the public chat configuration for a namespace |
| `config` | `get-log` | admin | v1 | GET | `/chat/v1/admin/config/log` | Retrieves the log configuration |
| `config` | `import` | admin | v1 | POST | `/chat/v1/admin/config/namespaces/{namespace}/import` | Imports chat configuration from a JSON file |
| `config` | `list` | admin | v1 | GET | `/chat/v1/admin/config` | Retrieves configuration for all namespaces |
| `config` | `update` | admin | v1 | PUT | `/chat/v1/admin/config/namespaces/{namespace}` | Updates the chat configuration for a namespace |
| `config` | `update-log` | admin | v1 | PATCH | `/chat/v1/admin/config/log` | Updates the log configuration |
| `inbox` | `create-category` | admin | v1 | POST | `/chat/v1/admin/inbox/namespaces/{namespace}/categories` | Adds an inbox category |
| `inbox` | `create-message` | admin | v1 | POST | `/chat/v1/admin/inbox/namespaces/{namespace}/messages` | Creates an inbox message |
| `inbox` | `delete-category` | admin | v1 | DELETE | `/chat/v1/admin/inbox/namespaces/{namespace}/categories/{category}` | Deletes an inbox category |
| `inbox` | `delete-message` | admin | v1 | DELETE | `/chat/v1/admin/inbox/namespaces/{namespace}/message/{messageId}` | Deletes an inbox message |
| `inbox` | `get-category-schema` | admin | v1 | GET | `/chat/v1/admin/inbox/namespaces/{namespace}/categories/{category}/schema.json` | Retrieves the schema for an inbox category |
| `inbox` | `list-categories` | admin | v1 | GET | `/chat/v1/admin/inbox/namespaces/{namespace}/categories` | Retrieves inbox categories for a namespace |
| `inbox` | `list-kafka-topics` | admin | v1 | GET | `/chat/v1/admin/inbox/namespaces/{namespace}/list/topic/kafka` | Lists Kafka topics for inbox notifications |
| `inbox` | `list-messages` | admin | v1 | GET | `/chat/v1/admin/inbox/namespaces/{namespace}/messages` | Lists inbox messages for a namespace |
| `inbox` | `list-stats` | admin | v1 | GET | `/chat/v1/admin/inbox/namespaces/{namespace}/stats` | Retrieves inbox statistics for specified messages |
| `inbox` | `list-users` | admin | v1 | GET | `/chat/v1/admin/inbox/namespaces/{namespace}/messages/{inbox}/users` | Lists users who received an inbox message |
| `inbox` | `send-message` | admin | v1 | POST | `/chat/v1/admin/inbox/namespaces/{namespace}/messages/{messageId}/send` | Sends an inbox message to its recipients |
| `inbox` | `unsend-message` | admin | v1 | POST | `/chat/v1/admin/inbox/namespaces/{namespace}/messages/{inbox}/unsend` | Retracts an inbox message from specified recipients |
| `inbox` | `update-category` | admin | v1 | PATCH | `/chat/v1/admin/inbox/namespaces/{namespace}/categories/{category}` | Updates an inbox category |
| `inbox` | `update-message` | admin | v1 | PATCH | `/chat/v1/admin/inbox/namespaces/{namespace}/messages/{messageId}` | Updates an inbox message |
| `moderation` | `delete-snapshot` | admin | v1 | DELETE | `/chat/v1/admin/namespaces/{namespace}/snapshot/{chatId}` | Deletes a chat snapshot |
| `moderation` | `get-snapshot` | admin | v1 | GET | `/chat/v1/admin/namespaces/{namespace}/snapshot/{chatId}` | Retrieves a chat snapshot |
| `moderation` | `get-snapshot` | public | v1 | GET | `/chat/v1/public/namespaces/{namespace}/topic/{topic}/snapshot/{chatId}` | Retrieves a chat snapshot |
| `profanity` | `bulk-create` | admin | v1 | POST | `/chat/v1/admin/profanity/namespaces/{namespace}/dictionary/bulk` | Adds profanity words to the namespace dictionary in bulk |
| `profanity` | `create` | admin | v1 | POST | `/chat/v1/admin/profanity/namespaces/{namespace}/dictionary` | Adds a profanity word to the namespace dictionary |
| `profanity` | `delete` | admin | v1 | DELETE | `/chat/v1/admin/profanity/namespaces/{namespace}/dictionary/{id}` | Deletes a profanity word from the dictionary |
| `profanity` | `export` | admin | v1 | GET | `/chat/v1/admin/profanity/namespaces/{namespace}/dictionary/export` | Exports profanity words from the dictionary |
| `profanity` | `import` | admin | v1 | POST | `/chat/v1/admin/profanity/namespaces/{namespace}/dictionary/import` | Imports profanity words from a file |
| `profanity` | `list-groups` | admin | v1 | GET | `/chat/v1/admin/profanity/namespaces/{namespace}/dictionary/group` | Lists profanity word groups |
| `profanity` | `query` | admin | v1 | GET | `/chat/v1/admin/profanity/namespaces/{namespace}/dictionary` | Queries the profanity word dictionary |
| `profanity` | `update` | admin | v1 | PUT | `/chat/v1/admin/profanity/namespaces/{namespace}/dictionary/{id}` | Updates a profanity word in the dictionary |
| `service-messages` | `get` | public | v1 | GET | `/chat/v1/messages` | Retrieves service-level messages |
| `topics` | `add-member` | admin | v1 | POST | `/chat/admin/namespaces/{namespace}/topic/{topic}/user/{userId}` | Adds a user to a topic |
| `topics` | `ban-members` | admin | v1 | POST | `/chat/admin/namespaces/{namespace}/topic/{topic}/ban-members` | Bans users from a group topic |
| `topics` | `ban-members` | public | v1 | POST | `/chat/public/namespaces/{namespace}/topic/{topic}/ban-members` | Bans members from a group topic |
| `topics` | `create` | admin | v1 | POST | `/chat/admin/namespaces/{namespace}/topic` | Creates a group chat topic |
| `topics` | `create-namespace` | admin | v1 | POST | `/chat/admin/namespaces/{namespace}/namespace-topic` | Creates a namespace group topic |
| `topics` | `delete` | admin | v1 | DELETE | `/chat/admin/namespaces/{namespace}/topic/{topic}` | Deletes a group chat topic |
| `topics` | `delete` | public | v1 | DELETE | `/chat/public/namespaces/{namespace}/topic/{topic}/chats/{chatId}` | Deletes a chat message |
| `topics` | `delete-message` | admin | v1 | DELETE | `/chat/admin/namespaces/{namespace}/topic/{topic}/chats/{chatId}` | Deletes a chat message from a topic |
| `topics` | `filter-message` | admin | v1 | POST | `/chat/admin/namespaces/{namespace}/chat/filter` | Returns a filtered chat message with profanity analysis |
| `topics` | `get` | admin | v1 | GET | `/chat/admin/namespaces/{namespace}/topic` | Lists chat topics in a namespace |
| `topics` | `get-channel-info` | admin | v1 | GET | `/chat/admin/namespaces/{namespace}/topic/{topic}/channel` | Retrieves details of a channel topic |
| `topics` | `get-channel-summary` | admin | v1 | GET | `/chat/admin/namespaces/{namespace}/topic/channel/summary` | Retrieves a summary of channel topics in a namespace |
| `topics` | `get-history` | admin | v1 | GET | `/chat/admin/namespaces/{namespace}/chats` | Retrieves chat message history across all topics in a namespace |
| `topics` | `get-history` | public | v1 | GET | `/chat/public/namespaces/{namespace}/topic/{topic}/chats` | Retrieves chat message history for a topic |
| `topics` | `list` | admin | v1 | GET | `/chat/admin/namespaces/{namespace}/topics` | Lists topics in a namespace |
| `topics` | `list` | public | v1 | GET | `/chat/public/namespaces/{namespace}/topic` | Lists chat topics in a namespace |
| `topics` | `list-channels` | admin | v1 | GET | `/chat/admin/namespaces/{namespace}/topic/channel` | Lists channel chat topics in a namespace |
| `topics` | `list-joined` | admin | v1 | GET | `/chat/admin/namespaces/{namespace}/users/{userId}/topics` | Lists topics a user belongs to |
| `topics` | `list-members` | admin | v1 | GET | `/chat/admin/namespaces/{namespace}/topic/{topic}/members` | Lists members of a topic |
| `topics` | `list-muted` | public | v1 | GET | `/chat/public/namespaces/{namespace}/muted` | Retrieves topics where the current user is muted |
| `topics` | `list-shards` | admin | v1 | GET | `/chat/admin/namespaces/{namespace}/topic/{topic}/shards` | Lists shards for a topic |
| `topics` | `mute-member` | public | v1 | PUT | `/chat/public/namespaces/{namespace}/topic/{topic}/mute` | Mutes a topic member for the given duration |
| `topics` | `remove-member` | admin | v1 | DELETE | `/chat/admin/namespaces/{namespace}/topic/{topic}/user/{userId}` | Removes a user from a topic |
| `topics` | `search-log` | admin | v1 | GET | `/chat/admin/namespaces/{namespace}/topic/log` | Queries the topic activity log for a namespace |
| `topics` | `send` | admin | v1 | POST | `/chat/admin/namespaces/{namespace}/topic/{topic}/chats` | Sends a system message to a group topic |
| `topics` | `unban-members` | admin | v1 | POST | `/chat/admin/namespaces/{namespace}/topic/{topic}/unban-members` | Unbans users from a group topic |
| `topics` | `unban-members` | public | v1 | POST | `/chat/public/namespaces/{namespace}/topic/{topic}/unban-members` | Unbans members from a group topic |
| `topics` | `unmute-member` | public | v1 | PUT | `/chat/public/namespaces/{namespace}/topic/{topic}/unmute` | Removes the mute from a topic member |
| `topics` | `update` | admin | v1 | PUT | `/chat/admin/namespaces/{namespace}/topic/{topic}` | Updates group topic information |

## cloud-save

- Spec name: `cloudsave`
- Resources: 8
- Operations: 97

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `game-files` | `bulk-get` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/binaries/bulk` | Retrieves game binary records in bulk |
| `game-files` | `create` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/binaries` | Creates a game binary record |
| `game-files` | `create` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/binaries` | Creates a game binary record |
| `game-files` | `delete` | admin | v1 | DELETE | `/cloudsave/v1/admin/namespaces/{namespace}/binaries/{key}` | Deletes a game binary record |
| `game-files` | `delete` | public | v1 | DELETE | `/cloudsave/v1/namespaces/{namespace}/binaries/{key}` | Deletes a game binary record |
| `game-files` | `delete-ttl` | admin | v1 | DELETE | `/cloudsave/v1/admin/namespaces/{namespace}/binaries/{key}/ttl` | Deletes the TTL configuration for a game binary record |
| `game-files` | `generate-upload-url` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/binaries/{key}/presigned` | Generates a presigned URL for uploading a game binary record |
| `game-files` | `generate-upload-url` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/binaries/{key}/presigned` | Generates a presigned URL for uploading a game binary record |
| `game-files` | `get` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/binaries/{key}` | Retrieves a game binary record by key |
| `game-files` | `get` | public | v1 | GET | `/cloudsave/v1/namespaces/{namespace}/binaries/{key}` | Retrieves a game binary record |
| `game-files` | `list` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/binaries` | Lists game binary records |
| `game-files` | `list` | public | v1 | GET | `/cloudsave/v1/namespaces/{namespace}/binaries` | Lists game binary records |
| `game-files` | `update` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/binaries/{key}` | Updates a game binary record file |
| `game-files` | `update` | public | v1 | PUT | `/cloudsave/v1/namespaces/{namespace}/binaries/{key}` | Updates a game binary record file |
| `game-files` | `update-metadata` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/binaries/{key}/metadata` | Updates game binary record metadata |
| `game-records` | `bulk-get` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/records/bulk` | Retrieves game records in bulk |
| `game-records` | `create` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/records/{key}` | Creates or appends a game record |
| `game-records` | `create` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/records/{key}` | Creates or appends a game record |
| `game-records` | `delete` | admin | v1 | DELETE | `/cloudsave/v1/admin/namespaces/{namespace}/records/{key}` | Deletes a game record |
| `game-records` | `delete` | public | v1 | DELETE | `/cloudsave/v1/namespaces/{namespace}/records/{key}` | Deletes a game record |
| `game-records` | `delete-ttl` | admin | v1 | DELETE | `/cloudsave/v1/admin/namespaces/{namespace}/records/{key}/ttl` | Deletes the TTL configuration for a game record |
| `game-records` | `get` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/records/{key}` | Retrieves a game record by key |
| `game-records` | `get` | public | v1 | GET | `/cloudsave/v1/namespaces/{namespace}/records/{key}` | Retrieves a game record |
| `game-records` | `list` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/records` | Lists game record keys |
| `game-records` | `update` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/records/{key}` | Creates or replaces a game record |
| `game-records` | `update` | public | v1 | PUT | `/cloudsave/v1/namespaces/{namespace}/records/{key}` | Creates or replaces a game record |
| `game-records` | `update-concurrent` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/concurrent/records/{key}` | Creates or replaces a game record with optimistic concurrency control |
| `game-records` | `update-concurrent` | public | v1 | PUT | `/cloudsave/v1/namespaces/{namespace}/concurrent/records/{key}` | Creates or replaces a game record with optimistic concurrency control |
| `game-records-protected` | `bulk-get` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/adminrecords/bulk` | Retrieves admin game records by keys in bulk |
| `game-records-protected` | `create` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/adminrecords/{key}` | Creates or appends an admin game record |
| `game-records-protected` | `delete` | admin | v1 | DELETE | `/cloudsave/v1/admin/namespaces/{namespace}/adminrecords/{key}` | Deletes an admin game record |
| `game-records-protected` | `delete-ttl` | admin | v1 | DELETE | `/cloudsave/v1/admin/namespaces/{namespace}/adminrecords/{key}/ttl` | Deletes the TTL configuration for an admin game record |
| `game-records-protected` | `get` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/adminrecords/{key}` | Retrieves an admin game record by key |
| `game-records-protected` | `list` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/adminrecords` | Lists admin game record keys |
| `game-records-protected` | `update` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/adminrecords/{key}` | Creates or replaces an admin game record |
| `game-records-protected` | `update-concurrent` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/concurrent/adminrecords/{key}` | Creates or replaces an admin game record with optimistic concurrency control |
| `plugin-config` | `create` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/plugins` | Creates the plugin configuration |
| `plugin-config` | `delete` | admin | v1 | DELETE | `/cloudsave/v1/admin/namespaces/{namespace}/plugins` | Deletes the plugin configuration |
| `plugin-config` | `get` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/plugins` | Retrieves the plugin configuration |
| `plugin-config` | `update` | admin | v1 | PATCH | `/cloudsave/v1/admin/namespaces/{namespace}/plugins` | Updates the plugin configuration |
| `tags` | `create` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/tags` | Creates a tag |
| `tags` | `delete` | admin | v1 | DELETE | `/cloudsave/v1/admin/namespaces/{namespace}/tags/{tag}` | Deletes a tag |
| `tags` | `list` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/tags` | Lists available tags |
| `tags` | `list` | public | v1 | GET | `/cloudsave/v1/namespaces/{namespace}/tags` | Lists available tags |
| `user-files` | `bulk-get-my` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/users/me/binaries/bulk` | Retrieves binary records for the current user in bulk |
| `user-files` | `bulk-get-public` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/binaries/public/bulk` | Retrieves public binary records for a player in bulk |
| `user-files` | `bulk-get-public-by-key` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/users/bulk/binaries/{key}/public` | Retrieves public binary records for players in bulk |
| `user-files` | `create` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/binaries` | Creates a player binary record |
| `user-files` | `create` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/binaries` | Creates a player binary record |
| `user-files` | `delete` | admin | v1 | DELETE | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/binaries/{key}` | Deletes a player binary record |
| `user-files` | `delete` | public | v1 | DELETE | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/binaries/{key}` | Deletes a player binary record |
| `user-files` | `generate-upload-url` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/binaries/{key}/presigned` | Generates a presigned URL for uploading a player binary record |
| `user-files` | `generate-upload-url` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/binaries/{key}/presigned` | Generates a presigned URL for uploading a player binary record |
| `user-files` | `get` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/binaries/{key}` | Retrieves a player binary record by key |
| `user-files` | `get` | public | v1 | GET | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/binaries/{key}` | Retrieves a player binary record |
| `user-files` | `get-public` | public | v1 | GET | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/binaries/{key}/public` | Retrieves a player's public binary record |
| `user-files` | `list` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/binaries` | Lists player binary records |
| `user-files` | `list` | public | v1 | GET | `/cloudsave/v1/namespaces/{namespace}/users/me/binaries` | Lists my binary records |
| `user-files` | `list-public` | public | v1 | GET | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/binaries/public` | Lists another player's public binary records |
| `user-files` | `update` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/binaries/{key}` | Updates a player binary record file |
| `user-files` | `update` | public | v1 | PUT | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/binaries/{key}` | Updates a player binary record file |
| `user-files` | `update-metadata` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/binaries/{key}/metadata` | Updates player binary record metadata |
| `user-files` | `update-metadata` | public | v1 | PUT | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/binaries/{key}/metadata` | Updates the metadata of a player binary record |
| `user-records` | `bulk-get` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/records/bulk` | Retrieves player records by record keys in bulk |
| `user-records` | `bulk-get-by-key` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/users/records/{key}/bulk` | Retrieves player records by user IDs in bulk |
| `user-records` | `bulk-get-my` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/users/me/records/bulk` | Retrieves records for the current user in bulk |
| `user-records` | `bulk-get-public` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/records/public/bulk` | Retrieves public records for a player in bulk |
| `user-records` | `bulk-get-public-by-key` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/users/bulk/records/{key}/public` | Retrieves public records for players in bulk |
| `user-records` | `bulk-get-size` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/users/bulk/records/size` | Retrieves player record sizes in bulk |
| `user-records` | `bulk-update` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/records/bulk` | Creates or replaces player records in bulk |
| `user-records` | `bulk-update-by-key` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/users/records/{key}/bulk` | Creates or replaces player records in bulk |
| `user-records` | `create` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/records/{key}` | Creates or appends a player record |
| `user-records` | `create` | public | v1 | POST | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/records/{key}` | Creates or appends to a player record |
| `user-records` | `delete` | admin | v1 | DELETE | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/records/{key}` | Deletes a player record |
| `user-records` | `delete` | public | v1 | DELETE | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/records/{key}` | Deletes a player record |
| `user-records` | `get` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/records/{key}` | Retrieves a player record by key |
| `user-records` | `get` | public | v1 | GET | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/records/{key}` | Retrieves a player record |
| `user-records` | `get-public` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/records/{key}/public` | Retrieves a player public record |
| `user-records` | `get-public` | public | v1 | GET | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/records/{key}/public` | Retrieves a player's public record |
| `user-records` | `get-size` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/records/{key}/size` | Retrieves the size of a player record |
| `user-records` | `list-by-user-id` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/records` | Lists player record keys |
| `user-records` | `list-my` | public | v1 | GET | `/cloudsave/v1/namespaces/{namespace}/users/me/records` | Lists the current user's record keys |
| `user-records` | `list-public` | public | v1 | GET | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/records/public` | Lists public record keys for a player |
| `user-records` | `update` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/records/{key}` | Creates or replaces a player record |
| `user-records` | `update` | public | v1 | PUT | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/records/{key}` | Creates or replaces a player record |
| `user-records` | `update-concurrent` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/concurrent/records/{key}` | Creates or replaces a player private record with optimistic concurrency control |
| `user-records` | `update-concurrent` | public | v1 | PUT | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/concurrent/records/{key}` | Creates or replaces a player record with optimistic concurrency control |
| `user-records` | `update-concurrent-public` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/concurrent/records/{key}/public` | Creates or replaces a player public record with optimistic concurrency control |
| `user-records` | `update-concurrent-public` | public | v1 | PUT | `/cloudsave/v1/namespaces/{namespace}/users/{userId}/concurrent/records/{key}/public` | Creates or replaces a public player record with optimistic concurrency control |
| `user-records-protected` | `bulk-get` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/adminrecords/bulk` | Retrieves admin player records by key in bulk |
| `user-records-protected` | `bulk-get-by-key` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/users/adminrecords/{key}/bulk` | Retrieves admin player records by user IDs in bulk |
| `user-records-protected` | `create` | admin | v1 | POST | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/adminrecords/{key}` | Creates or appends an admin player record |
| `user-records-protected` | `delete` | admin | v1 | DELETE | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/adminrecords/{key}` | Deletes an admin player record |
| `user-records-protected` | `get` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/adminrecords/{key}` | Retrieves an admin player record by key |
| `user-records-protected` | `list` | admin | v1 | GET | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/adminrecords` | Lists admin player record keys |
| `user-records-protected` | `update` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/adminrecords/{key}` | Creates or replaces an admin player record |
| `user-records-protected` | `update-concurrent` | admin | v1 | PUT | `/cloudsave/v1/admin/namespaces/{namespace}/users/{userId}/concurrent/adminrecords/{key}` | Creates or replaces an admin player record with optimistic concurrency control |

## csm

- Spec name: `csm`
- Resources: 9
- Operations: 52

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `apps` | `create` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/apps/{app}` | Creates a new extend app |
| `apps` | `delete` | admin | v2 | DELETE | `/csm/v2/admin/namespaces/{namespace}/apps/{app}` | Deletes the extend app by name |
| `apps` | `get` | admin | v2 | GET | `/csm/v2/admin/namespaces/{namespace}/apps/{app}` | Retrieves the extend app by name |
| `apps` | `get-release-info` | admin | v1 | GET | `/csm/v1/admin/namespaces/{namespace}/apps/{app}/release` | Gets the Latest Release Version info of this App |
| `apps` | `list` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/apps` | Lists extend apps in the given game namespace |
| `apps` | `request-resource-limit-increase` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/resources/form` | Submits a request to increase the app resource limits |
| `apps` | `start` | admin | v2 | PUT | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/start` | Starts the Application |
| `apps` | `stop` | admin | v2 | PUT | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/stop` | Stops the Application |
| `apps` | `update` | admin | v2 | PATCH | `/csm/v2/admin/namespaces/{namespace}/apps/{app}` | Updates the app configuration |
| `apps` | `update-resource` | admin | v2 | PATCH | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/resources` | Updates the app resource allocation |
| `config` | `create-secret` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/secrets` | Saves an environment secret |
| `config` | `create-variable` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/variables` | Saves an environment variable |
| `config` | `delete-secret` | admin | v2 | DELETE | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/secrets/{configId}` | Deletes an environment secret |
| `config` | `delete-variable` | admin | v2 | DELETE | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/variables/{configId}` | Deletes an environment variable |
| `config` | `list-secrets` | admin | v2 | GET | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/secrets` | Lists environment secrets for the app |
| `config` | `list-variables` | admin | v2 | GET | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/variables` | Lists environment variables for the app |
| `config` | `update-secret` | admin | v2 | PUT | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/secrets/{configId}` | Updates an environment secret |
| `config` | `update-variable` | admin | v2 | PUT | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/variables/{configId}` | Updates an environment variable |
| `deployments` | `create` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/deployments` | Creates Deployment |
| `deployments` | `delete` | admin | v2 | DELETE | `/csm/v2/admin/namespaces/{namespace}/deployments/{deploymentId}` | Deletes a deployment by ID |
| `deployments` | `get` | admin | v2 | GET | `/csm/v2/admin/namespaces/{namespace}/deployments/{deploymentId}` | Retrieves a deployment by ID |
| `deployments` | `list` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/deployments` | Fetches the List of Deployments |
| `images` | `delete` | admin | v2 | DELETE | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/images` | Deletes app images |
| `images` | `list` | admin | v2 | GET | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/images` | Lists container images for the app |
| `nosql` | `create` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/nosql/databases` | Creates NoSQL Database for Extend App |
| `nosql` | `create-cluster` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/nosql/clusters` | Creates NoSQL Cluster |
| `nosql` | `create-credentials` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/nosql/crendentials` | Creates a new database credential for the customer |
| `nosql` | `delete` | admin | v2 | DELETE | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/nosql/databases` | Deletes NoSQL Database for Extend App |
| `nosql` | `delete-cluster` | admin | v2 | DELETE | `/csm/v2/admin/namespaces/{namespace}/nosql/clusters` | Deletes the NoSQL cluster |
| `nosql` | `get` | admin | v2 | GET | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/nosql/databases` | Retrieves the NoSQL database for the extend app |
| `nosql` | `get-access-tunnel` | admin | v2 | GET | `/csm/v2/admin/namespaces/{namespace}/nosql/tunnels` | Retrieves the NoSQL access tunnel information |
| `nosql` | `get-cluster` | admin | v2 | GET | `/csm/v2/admin/namespaces/{namespace}/nosql/clusters` | Retrieves NoSQL cluster information |
| `nosql` | `list-apps` | admin | v2 | GET | `/csm/v2/admin/namespaces/{studioName}/nosql/{resourceId}/apps` | Lists extend apps using the NoSQL cluster |
| `nosql` | `start-cluster` | admin | v2 | PUT | `/csm/v2/admin/namespaces/{namespace}/nosql/clusters/start` | Starts the NoSQL cluster |
| `nosql` | `stop-cluster` | admin | v2 | PUT | `/csm/v2/admin/namespaces/{namespace}/nosql/clusters/stop` | Stops the NoSQL cluster |
| `nosql` | `update-cluster` | admin | v2 | PUT | `/csm/v2/admin/namespaces/{namespace}/nosql/clusters` | Updates the NoSQL cluster configuration |
| `resource-limits` | `list` | admin | v2 | GET | `/csm/v2/admin/namespaces/{namespace}/resources/limits` | Retrieves configurable resource limits for extend apps |
| `service-messages` | `list` | public | v1 | GET | `/csm/v1/messages` | Retrieves service messages |
| `subscriptions` | `add` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/subscriptions` | Subscribes users to app notifications |
| `subscriptions` | `add-my` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/subscriptions/me` | Subscribes the authenticated user to app notifications |
| `subscriptions` | `delete` | admin | v3 | DELETE | `/csm/v3/admin/namespaces/{namespace}/apps/{app}/subscriptions` | Removes a user's notification subscription by user ID or email |
| `subscriptions` | `delete-user` | admin | v2 | DELETE | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/subscriptions/users/{userId}` | Removes a user's notification subscription |
| `subscriptions` | `get-my` | admin | v2 | GET | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/subscriptions/me` | Retrieves the current user's notification subscription status |
| `subscriptions` | `list-subscribers` | admin | v2 | GET | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/subscriptions` | Lists app notification subscribers |
| `subscriptions` | `list-subscribers` | admin | v3 | GET | `/csm/v3/admin/namespaces/{namespace}/apps/{app}/subscriptions` | Lists app notification subscribers |
| `subscriptions` | `remove-my` | admin | v2 | DELETE | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/subscriptions/me` | Unsubscribes the authenticated user from app notifications |
| `subscriptions` | `update` | admin | v2 | PUT | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/subscriptions` | Updates notification subscriptions for the specified users |
| `topics` | `create` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/asyncmessaging/topics` | Creates an async messaging topic |
| `topics` | `delete` | admin | v2 | DELETE | `/csm/v2/admin/namespaces/{namespace}/asyncmessaging/topics/{topicName}` | Deletes an async messaging topic |
| `topics` | `list` | admin | v2 | GET | `/csm/v2/admin/namespaces/{namespace}/asyncmessaging/topics` | Lists async messaging topics |
| `topics` | `subscribe` | admin | v2 | POST | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/asyncmessaging/topics/subscriptions` | Subscribes the app to the specified async messaging topics |
| `topics` | `unsubscribe` | admin | v2 | DELETE | `/csm/v2/admin/namespaces/{namespace}/apps/{app}/asyncmessaging/topics/{topicName}/subscriptions` | Unsubscribes the app from an async messaging topic |

## game-telemetry

- Spec name: `gametelemetry`
- Resources: 3
- Operations: 4

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `events` | `search` | admin | v1 | GET | `/game-telemetry/v1/admin/namespaces/{namespace}/events` | Retrieves telemetry events for a namespace |
| `namespaces` | `list` | admin | v1 | GET | `/game-telemetry/v1/admin/namespaces` | Retrieves all game telemetry namespaces available to the caller |
| `steam-playtime` | `get` | public | v1 | GET | `/game-telemetry/v1/protected/steamIds/{steamId}/playtime` | Retrieves a player's total Steam playtime for a specific game |
| `steam-playtime` | `update` | public | v1 | PUT | `/game-telemetry/v1/protected/steamIds/{steamId}/playtime/{playtime}` | Updates a player's cached Steam playtime for a specific game |

## gdpr

- Spec name: `gdpr`
- Resources: 6
- Operations: 42

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `config` | `add-admin-email` | admin | v1 | POST | `/gdpr/admin/namespaces/{namespace}/emails/configurations` | Adds admin email addresses to the personal data notification list |
| `config` | `get-admin-email` | admin | v1 | GET | `/gdpr/admin/namespaces/{namespace}/emails/configurations` | Retrieves admin email address configurations for the namespace |
| `config` | `list-services` | admin | v1 | GET | `/gdpr/admin/namespaces/{namespace}/services/configurations` | Retrieves registered service configurations for the namespace |
| `config` | `remove-admin-email` | admin | v1 | DELETE | `/gdpr/admin/namespaces/{namespace}/emails/configurations` | Deletes admin email addresses from the personal data notification list |
| `config` | `reset-services` | admin | v1 | DELETE | `/gdpr/admin/namespaces/{namespace}/services/configurations/reset` | Resets registered service configurations to defaults |
| `config` | `update-admin-email` | admin | v1 | PUT | `/gdpr/admin/namespaces/{namespace}/emails/configurations` | Updates admin email addresses in the personal data notification list |
| `config` | `update-services` | admin | v1 | PUT | `/gdpr/admin/namespaces/{namespace}/services/configurations` | Updates registered service configurations for the namespace |
| `platform-closure` | `delete-client` | admin | v1 | DELETE | `/gdpr/admin/namespaces/{namespace}/platforms/{platform}/closure/client` | Deletes the account closure client configuration for the specified platform |
| `platform-closure` | `get-client` | admin | v1 | GET | `/gdpr/admin/namespaces/{namespace}/platforms/{platform}/closure/client` | Retrieves the account closure client configuration for the specified platform |
| `platform-closure` | `get-history` | admin | v1 | GET | `/gdpr/admin/namespaces/{namespace}/users/platforms/closure/histories` | Retrieves platform account closure histories across users in the namespace |
| `platform-closure` | `get-services` | admin | v1 | GET | `/gdpr/admin/namespaces/{namespace}/services/platforms/closure/config` | Retrieves platform account closure service configurations for the namespace |
| `platform-closure` | `list-clients` | admin | v1 | GET | `/gdpr/admin/namespaces/{namespace}/platforms/closure/clients` | Retrieves platform account closure client configurations |
| `platform-closure` | `mock-closure-event` | admin | v1 | POST | `/gdpr/admin/namespaces/{namespace}/platforms/{platform}/closure/mock` | Creates a mock account closure event for the specified platform |
| `platform-closure` | `reset-services` | admin | v1 | DELETE | `/gdpr/admin/namespaces/{namespace}/services/platforms/closure/config` | Resets platform account closure service configurations to defaults |
| `platform-closure` | `update-client` | admin | v1 | POST | `/gdpr/admin/namespaces/{namespace}/platforms/{platform}/closure/client` | Updates the account closure client configuration for the specified platform |
| `platform-closure` | `update-services` | admin | v1 | PUT | `/gdpr/admin/namespaces/{namespace}/services/platforms/closure/config` | Updates platform account closure service configurations for the namespace |
| `platform-closure` | `validate-xbox-business-partner-certificate` | admin | v1 | POST | `/gdpr/admin/namespaces/{namespace}/platforms/xbox/closure/cert/validation` | Validates the Xbox Business Partner certificate file for account closure |
| `user-access-requests` | `cancel` | admin | v1 | DELETE | `/gdpr/admin/namespaces/{namespace}/users/{userId}/requests/{requestDate}` | Cancels a user's pending personal data request |
| `user-access-requests` | `cancel` | public | v1 | DELETE | `/gdpr/public/namespaces/{namespace}/users/{userId}/requests/{requestDate}` | Cancels a user's pending personal data request |
| `user-access-requests` | `generate-download-url` | admin | v1 | POST | `/gdpr/admin/namespaces/{namespace}/users/{userId}/requests/{requestDate}/generate` | Generates a personal data download URL for the specified request |
| `user-access-requests` | `generate-download-url` | public | v1 | POST | `/gdpr/public/namespaces/{namespace}/users/{userId}/requests/{requestDate}/generate` | Generates a personal data download URL for the specified request |
| `user-access-requests` | `list` | admin | v1 | GET | `/gdpr/admin/namespaces/{namespace}/users/{userId}/requests` | Retrieves personal data requests for a specific user |
| `user-access-requests` | `list` | public | v1 | GET | `/gdpr/public/namespaces/{namespace}/users/{userId}/requests` | Retrieves personal data requests for the specified user |
| `user-access-requests` | `list-all` | admin | v1 | GET | `/gdpr/admin/namespaces/{namespace}/requests` | Lists personal data requests across the namespace |
| `user-access-requests` | `submit` | admin | v1 | POST | `/gdpr/admin/namespaces/{namespace}/users/{userId}/requests` | Submits a personal data retrieval request for a user |
| `user-access-requests` | `submit` | public | v1 | POST | `/gdpr/public/namespaces/{namespace}/users/{userId}/requests` | Submits a personal data retrieval request for the specified user |
| `user-access-requests-s2s` | `generate-download-url` | admin | v1 | POST | `/gdpr/s2s/namespaces/{namespace}/users/{userId}/requests/{requestDate}/generate` | Generates a personal data download URL via server-to-server |
| `user-access-requests-s2s` | `get` | admin | v1 | GET | `/gdpr/s2s/namespaces/{namespace}/requests/{requestId}` | Retrieves a personal data request by its ID |
| `user-access-requests-s2s` | `list-finished` | admin | v1 | GET | `/gdpr/s2s/namespaces/{namespace}/requests/finished` | Lists finished personal data requests within a time range |
| `user-access-requests-s2s` | `submit` | admin | v1 | POST | `/gdpr/s2s/namespaces/{namespace}/users/{userId}/requests` | Submits a personal data retrieval request via server-to-server |
| `user-deletion-requests` | `cancel-all` | admin | v1 | DELETE | `/gdpr/admin/namespaces/{namespace}/users/{userId}/deletions` | Cancels a user's pending account deletion request |
| `user-deletion-requests` | `cancel-all` | public | v1 | DELETE | `/gdpr/public/namespaces/{namespace}/users/{userId}/deletions` | Cancels a user's pending account deletion request |
| `user-deletion-requests` | `cancel-all-my` | public | v1 | DELETE | `/gdpr/public/users/me/deletions` | Cancels the current user's pending account deletion request |
| `user-deletion-requests` | `get` | admin | v1 | GET | `/gdpr/admin/namespaces/{namespace}/users/{userId}/deletions` | Retrieves the account deletion request for a specific user |
| `user-deletion-requests` | `get` | public | v1 | GET | `/gdpr/public/namespaces/{namespace}/users/{userId}/deletions/status` | Retrieves the account deletion status for the specified user |
| `user-deletion-requests` | `get-my` | public | v1 | GET | `/gdpr/public/users/me/deletions/status` | Retrieves the current user's account deletion status |
| `user-deletion-requests` | `list-all` | admin | v1 | GET | `/gdpr/admin/namespaces/{namespace}/deletions` | Retrieves all account deletion requests for a specified date |
| `user-deletion-requests` | `submit` | admin | v1 | POST | `/gdpr/admin/namespaces/{namespace}/users/{userId}/deletions` | Submits an account deletion request for a specific user |
| `user-deletion-requests` | `submit` | public | v1 | POST | `/gdpr/public/namespaces/{namespace}/users/{userId}/deletions` | Submits an account deletion request for the specified user |
| `user-deletion-requests` | `submit-my` | public | v1 | POST | `/gdpr/public/users/me/deletions` | Submits an account deletion request for the current user |
| `user-deletion-requests-s2s` | `list-finished` | admin | v1 | GET | `/gdpr/s2s/namespaces/{namespace}/deletions/finished` | Lists finished account deletion requests within a time range |
| `user-deletion-requests-s2s` | `submit` | admin | v1 | POST | `/gdpr/s2s/namespaces/{namespace}/users/{userId}/deletions` | Submits a user's account deletion request via server-to-server |

## group

- Spec name: `group`
- Resources: 5
- Operations: 73

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `group-definitions` | `create` | admin | v1 | POST | `/group/v1/admin/namespaces/{namespace}/configuration` | Creates a new group configuration |
| `group-definitions` | `delete` | admin | v1 | DELETE | `/group/v1/admin/namespaces/{namespace}/configuration/{configurationCode}` | Deletes a group configuration |
| `group-definitions` | `delete-global-rule` | admin | v1 | DELETE | `/group/v1/admin/namespaces/{namespace}/configuration/{configurationCode}/rules/{allowedAction}` | Deletes a global rule from a group configuration |
| `group-definitions` | `get` | admin | v1 | GET | `/group/v1/admin/namespaces/{namespace}/configuration/{configurationCode}` | Retrieves a group configuration |
| `group-definitions` | `list` | admin | v1 | GET | `/group/v1/admin/namespaces/{namespace}/configuration` | Lists group configurations |
| `group-definitions` | `set-default` | admin | v1 | POST | `/group/v1/admin/namespaces/{namespace}/configuration/initiate` | Initiates the default group service configuration for the namespace |
| `group-definitions` | `update` | admin | v1 | PATCH | `/group/v1/admin/namespaces/{namespace}/configuration/{configurationCode}` | Updates an existing group configuration |
| `group-definitions` | `update-global-rule` | admin | v1 | PUT | `/group/v1/admin/namespaces/{namespace}/configuration/{configurationCode}/rules/{allowedAction}` | Updates a global rule in a group configuration |
| `groups` | `bulk-get` | admin | v2 | POST | `/group/v2/admin/namespaces/{namespace}/groups/bulk` | Retrieves groups by IDs |
| `groups` | `bulk-get` | public | v2 | POST | `/group/v2/public/namespaces/{namespace}/groups/bulk` | Retrieves groups by IDs |
| `groups` | `create` | public | v1 | POST | `/group/v1/public/namespaces/{namespace}/groups` | Creates a new group |
| `groups` | `create` | public | v2 | POST | `/group/v2/public/namespaces/{namespace}/groups` | Creates a new group |
| `groups` | `delete` | admin | v1 | DELETE | `/group/v1/admin/namespaces/{namespace}/groups/{groupId}` | Deletes an existing group |
| `groups` | `delete` | public | v1 | DELETE | `/group/v1/public/namespaces/{namespace}/groups/{groupId}` | Deletes a group |
| `groups` | `delete` | public | v2 | DELETE | `/group/v2/public/namespaces/{namespace}/groups/{groupId}` | Deletes a group |
| `groups` | `delete-predefined-rule` | public | v1 | DELETE | `/group/v1/public/namespaces/{namespace}/groups/{groupId}/rules/defined/{allowedAction}` | Deletes a predefined rule from a group |
| `groups` | `delete-predefined-rule` | public | v2 | DELETE | `/group/v2/public/namespaces/{namespace}/groups/{groupId}/rules/defined/{allowedAction}` | Deletes a predefined rule from a group |
| `groups` | `get` | admin | v1 | GET | `/group/v1/admin/namespaces/{namespace}/groups/{groupId}` | Retrieves a group by ID |
| `groups` | `get` | public | v1 | GET | `/group/v1/public/namespaces/{namespace}/groups/{groupId}` | Retrieves group information |
| `groups` | `list` | admin | v1 | GET | `/group/v1/admin/namespaces/{namespace}/groups` | Lists groups in a namespace |
| `groups` | `list` | public | v1 | GET | `/group/v1/public/namespaces/{namespace}/groups` | Lists public and open groups |
| `groups` | `set` | public | v1 | PUT | `/group/v1/public/namespaces/{namespace}/groups/{groupId}` | Replaces an existing group |
| `groups` | `set` | public | v2 | PUT | `/group/v2/public/namespaces/{namespace}/groups/{groupId}` | Replaces an existing group |
| `groups` | `update` | public | v1 | PATCH | `/group/v1/public/namespaces/{namespace}/groups/{groupId}` | Updates an existing group |
| `groups` | `update` | public | v2 | PATCH | `/group/v2/public/namespaces/{namespace}/groups/{groupId}` | Updates an existing group |
| `groups` | `update-custom-attributes` | public | v1 | PUT | `/group/v1/public/namespaces/{namespace}/groups/{groupId}/attributes/custom` | Replaces custom attributes for a group |
| `groups` | `update-custom-attributes` | public | v2 | PUT | `/group/v2/public/namespaces/{namespace}/groups/{groupId}/attributes/custom` | Replaces custom attributes for a group |
| `groups` | `update-custom-rule` | public | v1 | PUT | `/group/v1/public/namespaces/{namespace}/groups/{groupId}/rules/custom` | Updates custom rules for a group |
| `groups` | `update-custom-rule` | public | v2 | PUT | `/group/v2/public/namespaces/{namespace}/groups/{groupId}/rules/custom` | Updates custom rules for a group |
| `groups` | `update-predefined-rule` | public | v1 | PUT | `/group/v1/public/namespaces/{namespace}/groups/{groupId}/rules/defined/{allowedAction}` | Updates a predefined rule in a group |
| `groups` | `update-predefined-rule` | public | v2 | PUT | `/group/v2/public/namespaces/{namespace}/groups/{groupId}/rules/defined/{allowedAction}` | Updates a predefined rule in a group |
| `membership` | `get` | admin | v2 | GET | `/group/v2/admin/namespaces/{namespace}/users/{userId}/groups/{groupId}/status` | Retrieves a user's group membership status |
| `membership` | `get` | public | v1 | GET | `/group/v1/public/namespaces/{namespace}/users/{userId}` | Retrieves group membership information for a user |
| `membership` | `get` | public | v2 | GET | `/group/v2/public/namespaces/{namespace}/users/{userId}/groups/{groupId}/status` | Retrieves the membership status of a user in a group |
| `membership` | `kick` | public | v1 | POST | `/group/v1/public/namespaces/{namespace}/users/{userId}/kick` | Kicks a member from a group |
| `membership` | `kick` | public | v2 | POST | `/group/v2/public/namespaces/{namespace}/users/{userId}/groups/{groupId}/kick` | Kicks a member from the group |
| `membership` | `leave` | public | v1 | POST | `/group/v1/public/namespaces/{namespace}/leave` | Leaves the current group |
| `membership` | `leave` | public | v2 | POST | `/group/v2/public/namespaces/{namespace}/groups/{groupId}/leave` | Leaves the current group |
| `membership` | `list` | admin | v1 | GET | `/group/v1/admin/namespaces/{namespace}/groups/{groupId}/members` | Lists members of a group |
| `membership` | `list` | public | v1 | GET | `/group/v1/public/namespaces/{namespace}/groups/{groupId}/members` | Lists members of a group |
| `membership` | `list-joined` | admin | v2 | GET | `/group/v2/admin/namespaces/{namespace}/users/{userId}/groups` | Lists groups a user has joined |
| `membership` | `list-my-joined` | public | v2 | GET | `/group/v2/public/namespaces/{namespace}/users/me/groups` | Lists groups the current user has joined |
| `requests` | `accept-invitation` | public | v1 | POST | `/group/v1/public/namespaces/{namespace}/groups/{groupId}/invite/accept` | Accepts a group invitation |
| `requests` | `accept-invitation` | public | v2 | POST | `/group/v2/public/namespaces/{namespace}/groups/{groupId}/invite/accept` | Accepts a group invitation |
| `requests` | `accept-join` | public | v1 | POST | `/group/v1/public/namespaces/{namespace}/users/{userId}/join/accept` | Accepts a join request from a user |
| `requests` | `accept-join` | public | v2 | POST | `/group/v2/public/namespaces/{namespace}/users/{userId}/groups/{groupId}/join/accept` | Accepts a join request from a user |
| `requests` | `cancel-invitation` | public | v1 | POST | `/group/v1/public/namespaces/{namespace}/groups/{groupId}/join/cancel` | Cancels a join request |
| `requests` | `cancel-invitation` | public | v2 | POST | `/group/v2/public/namespaces/{namespace}/users/{userId}/groups/{groupId}/invite/cancel` | Cancels an invitation to a group member |
| `requests` | `invite` | public | v1 | POST | `/group/v1/public/namespaces/{namespace}/users/{userId}/invite` | Invites a user to a group |
| `requests` | `invite` | public | v2 | POST | `/group/v2/public/namespaces/{namespace}/users/{userId}/groups/{groupId}/invite` | Invites a user to a group |
| `requests` | `join` | public | v1 | POST | `/group/v1/public/namespaces/{namespace}/groups/{groupId}/join` | Joins a group or submits a join request |
| `requests` | `join` | public | v2 | POST | `/group/v2/public/namespaces/{namespace}/groups/{groupId}/join` | Joins a group or submits a join request |
| `requests` | `list-invitation` | public | v1 | GET | `/group/v1/public/namespaces/{namespace}/users/me/invite/request` | Lists group invitations for the current user |
| `requests` | `list-invitations` | public | v2 | GET | `/group/v2/public/namespaces/{namespace}/groups/{groupId}/invite/request` | Lists invite requests for a group |
| `requests` | `list-joins` | public | v1 | GET | `/group/v1/public/namespaces/{namespace}/groups/{groupId}/join/request` | Lists join requests for a group |
| `requests` | `list-joins` | public | v2 | GET | `/group/v2/public/namespaces/{namespace}/groups/{groupId}/join/request` | Lists join requests for a group |
| `requests` | `list-my-join-requests` | public | v2 | GET | `/group/v2/public/namespaces/{namespace}/users/me/join/request` | Lists the current user's join requests |
| `requests` | `reject-invitation` | public | v1 | POST | `/group/v1/public/namespaces/{namespace}/groups/{groupId}/invite/reject` | Rejects a group invitation |
| `requests` | `reject-invitation` | public | v2 | POST | `/group/v2/public/namespaces/{namespace}/groups/{groupId}/invite/reject` | Rejects a group invitation |
| `requests` | `reject-join` | public | v1 | POST | `/group/v1/public/namespaces/{namespace}/users/{userId}/join/reject` | Rejects a join request from a user |
| `requests` | `reject-join` | public | v2 | POST | `/group/v2/public/namespaces/{namespace}/users/{userId}/groups/{groupId}/join/reject` | Rejects a join request from a user |
| `roles` | `create` | admin | v1 | POST | `/group/v1/admin/namespaces/{namespace}/roles` | Creates a new member role |
| `roles` | `delete` | admin | v1 | DELETE | `/group/v1/admin/namespaces/{namespace}/roles/{memberRoleId}` | Deletes a member role |
| `roles` | `delete` | public | v1 | DELETE | `/group/v1/public/namespaces/{namespace}/roles/{memberRoleId}/members` | Removes a role from a group member |
| `roles` | `delete` | public | v2 | DELETE | `/group/v2/public/namespaces/{namespace}/roles/{memberRoleId}/groups/{groupId}/members` | Removes a role from a group member |
| `roles` | `get` | admin | v1 | GET | `/group/v1/admin/namespaces/{namespace}/roles/{memberRoleId}` | Retrieves a member role |
| `roles` | `list` | admin | v1 | GET | `/group/v1/admin/namespaces/{namespace}/roles` | Lists member roles |
| `roles` | `list` | public | v1 | GET | `/group/v1/public/namespaces/{namespace}/roles` | Lists member roles |
| `roles` | `list` | public | v2 | GET | `/group/v2/public/namespaces/{namespace}/roles` | Lists member roles |
| `roles` | `update` | admin | v1 | PATCH | `/group/v1/admin/namespaces/{namespace}/roles/{memberRoleId}` | Updates a member role |
| `roles` | `update` | public | v1 | POST | `/group/v1/public/namespaces/{namespace}/roles/{memberRoleId}/members` | Assigns a role to a group member |
| `roles` | `update` | public | v2 | POST | `/group/v2/public/namespaces/{namespace}/roles/{memberRoleId}/groups/{groupId}/members` | Assigns a role to a group member |
| `roles` | `update-permissions` | admin | v1 | PUT | `/group/v1/admin/namespaces/{namespace}/roles/{memberRoleId}/permissions` | Updates permissions for a member role |

## iam

- Spec name: `iam`
- Resources: 19
- Operations: 306

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `age-restrictions` | `get` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/agerestrictions/countries/{countryCode}` | Retrieves age restriction settings for a country |
| `age-restrictions` | `list` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/agerestrictions` | Retrieves age restriction status for a namespace |
| `age-restrictions` | `list-countries` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/agerestrictions/countries` | Lists country age restrictions for a namespace |
| `age-restrictions` | `update` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/agerestrictions` | Updates age restriction configuration for a namespace |
| `age-restrictions` | `update-country` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/agerestrictions/countries/{countryCode}` | Updates age restriction for a specific country |
| `allow-list` | `get` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/loginAllowlist` | Retrieves the login allowlist configuration |
| `allow-list` | `update` | admin | v3 | PUT | `/iam/v3/admin/namespaces/{namespace}/loginAllowlist` | Updates the login allowlist configuration |
| `bans` | `bulk-ban-users` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/bans/users` | Applies a ban to the specified users |
| `bans` | `bulk-unban-users` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/bans/users/disabled` | Removes bans from users in bulk |
| `bans` | `list` | admin | v3 | GET | `/iam/v3/admin/bans` | Lists all available ban types |
| `bans` | `list-banned-users` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/bans/users` | Lists users filtered by ban type |
| `bans` | `list-reasons` | admin | v3 | GET | `/iam/v3/admin/bans/reasons` | Lists all available ban reasons |
| `bans` | `list-types` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/bantypes` | Lists ban types available in a namespace |
| `client-config` | `delete-permission` | admin | v3 | DELETE | `/iam/v3/admin/clientConfig/permissions` | Deletes client configuration permissions by module and group |
| `client-config` | `list-permissions` | admin | v3 | GET | `/iam/v3/admin/clientConfig/permissions` | Lists available client configuration permissions |
| `client-config` | `list-templates` | admin | v3 | GET | `/iam/v3/admin/clientConfig/templates` | Lists available client configuration templates |
| `client-config` | `upsert-permissions` | admin | v3 | PUT | `/iam/v3/admin/clientConfig/permissions` | Updates or creates client permission modules |
| `clients` | `add-permissions` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/clients/{clientId}/permissions` | Adds permissions to an OAuth 2.0 client |
| `clients` | `bulk-update` | admin | v3 | PUT | `/iam/v3/admin/namespaces/{namespace}/clients` | Updates a set of OAuth 2.0 clients |
| `clients` | `create` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/clients` | Creates a new OAuth 2.0 client |
| `clients` | `delete` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/clients/{clientId}` | Deletes an OAuth 2.0 client |
| `clients` | `delete-permission` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/clients/{clientId}/permissions/{resource}/{action}` | Deletes a specific permission from an OAuth 2.0 client |
| `clients` | `get` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/clients/{clientId}` | Retrieves an OAuth 2.0 client by ID |
| `clients` | `list` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/clients` | Lists OAuth 2.0 clients in a namespace |
| `clients` | `update` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/clients/{clientId}` | Updates an OAuth 2.0 client |
| `clients` | `update-permissions` | admin | v3 | PUT | `/iam/v3/admin/namespaces/{namespace}/clients/{clientId}/permissions` | Replaces all permissions for an OAuth 2.0 client |
| `clients` | `update-secret` | admin | v3 | PUT | `/iam/v3/admin/namespaces/{namespace}/{clientId}/secret` | Updates the secret for an OAuth client |
| `config` | `get` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/config/{configKey}` | Retrieves a configuration value by key |
| `config` | `get` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/config/{configKey}` | Retrieves a namespace configuration value |
| `config` | `get-public` | public | v3 | GET | `/iam/v3/config/public` | Retrieves public system configuration values |
| `countries` | `get-deny-list` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/countries/blacklist` | Retrieves the country blacklist |
| `countries` | `list` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/countries` | Lists available countries |
| `countries` | `list` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/countries` | Retrieves the list of countries |
| `countries` | `update-deny-list` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/countries/blacklist` | Updates the country blacklist |
| `devices` | `ban` | admin | v4 | POST | `/iam/v4/admin/namespaces/{namespace}/devices/bans` | Bans a device |
| `devices` | `generate-report` | admin | v4 | GET | `/iam/v4/admin/namespaces/{namespace}/devices/report` | Generates a device activity report |
| `devices` | `get-banned` | admin | v4 | GET | `/iam/v4/admin/namespaces/{namespace}/devices/bans/{banId}` | Retrieves a device ban configuration |
| `devices` | `list` | admin | v4 | GET | `/iam/v4/admin/namespaces/{namespace}/devices` | Retrieves devices a user has used to log in |
| `devices` | `list-banned` | admin | v4 | GET | `/iam/v4/admin/namespaces/{namespace}/devices/{deviceId}/bans` | Retrieves ban records for a device |
| `devices` | `list-banned-all` | admin | v4 | GET | `/iam/v4/admin/namespaces/{namespace}/devices/banned` | Retrieves banned devices with pagination |
| `devices` | `list-banned-user` | admin | v4 | GET | `/iam/v4/admin/namespaces/{namespace}/devices/bans` | Retrieves device bans for a user |
| `devices` | `list-types` | admin | v4 | GET | `/iam/v4/admin/namespaces/{namespace}/devices/types` | Retrieves available device types |
| `devices` | `list-users` | admin | v4 | GET | `/iam/v4/admin/namespaces/{namespace}/devices/{deviceId}/users` | Retrieves users who have logged in on a device |
| `devices` | `unban` | admin | v4 | PUT | `/iam/v4/admin/namespaces/{namespace}/devices/{deviceId}/unban` | Unbans a device |
| `devices` | `update-banned` | admin | v4 | PUT | `/iam/v4/admin/namespaces/{namespace}/devices/bans/{banId}` | Updates a device ban configuration |
| `input-validations` | `get` | public | v3 | GET | `/iam/v3/public/inputValidations/{field}` | Retrieves input validation configuration for a specific field |
| `input-validations` | `list` | admin | v3 | GET | `/iam/v3/admin/inputValidations` | Retrieves input validation configurations |
| `input-validations` | `list` | public | v3 | GET | `/iam/v3/public/inputValidations` | Retrieves input validation configurations |
| `input-validations` | `reset` | admin | v3 | DELETE | `/iam/v3/admin/inputValidations/{field}` | Resets input validation to default configurations |
| `input-validations` | `update` | admin | v3 | PUT | `/iam/v3/admin/inputValidations` | Updates input validation configurations |
| `oauth2` | `authenticate` | public | v3 | POST | `/iam/v3/authenticate` | Authenticates a user account |
| `oauth2` | `authenticate-platform` | public | v3 | GET | `/iam/v3/platforms/{platformId}/authenticate` | Authenticates a user via a third-party platform |
| `oauth2` | `authenticate-with-link` | public | v3 | POST | `/iam/v3/authenticateWithLink` | Authenticates a user and links a platform account |
| `oauth2` | `authenticate-with-link` | public | v4 | POST | `/iam/v4/oauth/authenticateWithLink` | Authenticates a user and links a platform account |
| `oauth2` | `authenticate-with-link-forward` | public | v3 | POST | `/iam/v3/authenticateWithLink/forward` | Authenticates a user and links a platform account with a redirect response |
| `oauth2` | `authorize` | public | v3 | GET | `/iam/v3/oauth/authorize` | Starts the OAuth 2.0 authorization flow |
| `oauth2` | `authorize-platform` | public | v3 | GET | `/iam/v3/oauth/platforms/{platformId}/authorize` | Generates a URL to request an auth code from a third-party platform |
| `oauth2` | `change-mfa-method` | public | v3 | POST | `/iam/v3/oauth/mfa/factor/change` | Updates the active 2FA method |
| `oauth2` | `check-token` | public | v3 | POST | `/iam/v3/oauth/introspect` | Returns information about an access token |
| `oauth2` | `exchange-token` | public | v3 | POST | `/iam/v3/token/exchange` | Generates a target token by exchanging a code |
| `oauth2` | `exchange-token` | public | v4 | POST | `/iam/v4/oauth/token/exchange` | Generates a target token by exchanging a one-time code |
| `oauth2` | `exchange-token-with-linking-code` | public | v3 | POST | `/iam/v3/link/token/exchange` | Issues a user token for a one-time linking code |
| `oauth2` | `generate-token-for-headless-account` | public | v3 | POST | `/iam/v3/headless/token` | Creates a headless account and returns a token |
| `oauth2` | `generate-token-for-headless-account` | public | v4 | POST | `/iam/v4/oauth/headless/token` | Creates a headless account after platform authentication and returns a token |
| `oauth2` | `get-country-location` | public | v3 | GET | `/iam/v3/location/country` | Retrieves the country based on request location |
| `oauth2` | `get-jwks` | public | v3 | GET | `/iam/v3/oauth/jwks` | Retrieves the JSON Web Key Set for JWT verification |
| `oauth2` | `get-platform-token` | admin | v3 | GET | `/iam/v3/oauth/admin/namespaces/{namespace}/users/{userId}/platforms/{platformId}/platformToken` | Retrieves a third-party platform token for a user |
| `oauth2` | `get-platform-token` | public | v3 | GET | `/iam/v3/oauth/namespaces/{namespace}/users/{userId}/platforms/{platformId}/platformToken` | Retrieves a third-party platform token for a user |
| `oauth2` | `grant-by-platform` | public | v3 | POST | `/iam/v3/oauth/platforms/{platformId}/token` | Grants an access token using a third-party platform token |
| `oauth2` | `grant-by-platform` | public | v4 | POST | `/iam/v4/oauth/platforms/{platformId}/token` | Generates an OAuth2 access token using a third-party platform credential |
| `oauth2` | `grant-token` | public | v3 | POST | `/iam/v3/oauth/token` | Generates an OAuth2 access token |
| `oauth2` | `grant-token` | public | v4 | POST | `/iam/v4/oauth/token` | Generates an OAuth2 access token |
| `oauth2` | `handle-web-upgrade-forward` | public | v3 | POST | `/iam/v3/upgrade/forward` | Handles the forward redirect for account upgrade |
| `oauth2` | `list-revocations` | public | v3 | GET | `/iam/v3/oauth/revocationlist` | Retrieves the token and user revocation list |
| `oauth2` | `login-simultaneous` | public | v3 | POST | `/iam/v3/oauth/simultaneousLogin` | Authenticates on two platforms simultaneously |
| `oauth2` | `login-simultaneous` | public | v4 | POST | `/iam/v4/oauth/simultaneousLogin` | Authenticates using simultaneous platform credentials |
| `oauth2` | `logout` | public | v3 | POST | `/iam/v3/logout` | Logs out the current user |
| `oauth2` | `request-one-time-linking-code` | public | v3 | POST | `/iam/v3/link/code/request` | Generates a one-time account linking code |
| `oauth2` | `request-token-exchange-code` | public | v3 | POST | `/iam/v3/namespace/{namespace}/token/request` | Requests a code for cross-namespace token exchange |
| `oauth2` | `revoke-token` | public | v3 | POST | `/iam/v3/oauth/revoke` | Revokes an access or refresh token |
| `oauth2` | `revoke-user` | admin | v3 | POST | `/iam/v3/oauth/admin/namespaces/{namespace}/users/{userId}/revoke` | Revokes user's tokens |
| `oauth2` | `send-mfa-code` | public | v3 | POST | `/iam/v3/oauth/mfa/code` | Sends a two-factor authentication code |
| `oauth2` | `validate-one-time-linking-code` | public | v3 | POST | `/iam/v3/link/code/validate` | Validates a one-time account linking code |
| `oauth2` | `verify-mfa-code` | public | v3 | POST | `/iam/v3/oauth/mfa/verify` | Verifies a 2FA code |
| `oauth2` | `verify-mfa-code` | public | v4 | POST | `/iam/v4/oauth/mfa/verify` | Verifies a 2FA code during login |
| `oauth2` | `verify-mfa-code-forward` | public | v3 | POST | `/iam/v3/oauth/mfa/verify/forward` | Verifies a 2FA code with redirect on result |
| `oauth2` | `verify-platform-token` | public | v3 | POST | `/iam/v3/platforms/{platformId}/token/verify` | Verifies a third-party platform token |
| `oauth2` | `verify-token` | public | v3 | POST | `/iam/v3/oauth/verify` | Verifies an OAuth2 token |
| `platform-credentials` | `check-availability` | admin | v3 | GET | `/iam/v3/admin/platforms/{platformId}/availability` | Checks the availability of a third-party platform |
| `platform-credentials` | `create` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/clients` | Adds third-party platform credentials |
| `platform-credentials` | `delete` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/clients` | Deletes third-party platform credentials |
| `platform-credentials` | `delete-domain` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/clients/domain` | Removes a third-party platform domain |
| `platform-credentials` | `get` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/clients` | Retrieves third-party platform credentials |
| `platform-credentials` | `list` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/platforms/all/clients` | Lists all third-party platform credentials |
| `platform-credentials` | `list-active` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/platforms/all/clients/active` | Lists all active third-party platform credentials |
| `platform-credentials` | `list-active-oidc` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/platforms/clients/active` | Retrieves active third-party platform credentials for public use |
| `platform-credentials` | `list-oidc` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/platforms/clients/oidc` | Retrieves the active OIDC platform credential for a client |
| `platform-credentials` | `set-domain` | admin | v3 | PUT | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/clients/domain` | Sets third-party platform domain configuration |
| `platform-credentials` | `update` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/clients` | Updates third-party platform credentials |
| `platform-credentials` | `update-domain` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/clients/domain` | Updates third-party platform domain configuration |
| `profile-update-strategies` | `list` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/profileUpdateStrategies` | Retrieves profile update strategies for a namespace |
| `profile-update-strategies` | `list` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/profileUpdateStrategies` | Retrieves profile update strategies for a namespace |
| `profile-update-strategies` | `update` | admin | v3 | PUT | `/iam/v3/admin/namespaces/{namespace}/profileUpdateStrategies` | Updates profile update strategies for a namespace |
| `role-override` | `get` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/roleoverride` | Retrieves role override configuration |
| `role-override` | `get-permissions` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/roleoverride/{roleId}/permissions` | Retrieves namespace permissions for a role override |
| `role-override` | `get-source` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/roleoverride/source` | Retrieves the source permission set for role overrides |
| `role-override` | `update` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/roleoverride` | Updates role override configuration |
| `role-override` | `update-status` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/roleoverride/status` | Updates the active status of role override configuration |
| `roles` | `add-permissions` | admin | v3 | POST | `/iam/v3/admin/roles/{roleId}/permissions` | Adds permissions to the role |
| `roles` | `add-permissions` | admin | v4 | POST | `/iam/v4/admin/roles/{roleId}/permissions` | Adds permissions to a role |
| `roles` | `assign-user` | admin | v4 | POST | `/iam/v4/admin/roles/{roleId}/users` | Assigns a user to a role |
| `roles` | `create` | admin | v3 | POST | `/iam/v3/admin/roles` | Creates a new role |
| `roles` | `create` | admin | v4 | POST | `/iam/v4/admin/roles` | Creates a new role |
| `roles` | `delete` | admin | v3 | DELETE | `/iam/v3/admin/roles/{roleId}` | Deletes a role |
| `roles` | `delete` | admin | v4 | DELETE | `/iam/v4/admin/roles/{roleId}` | Deletes a role |
| `roles` | `get` | admin | v3 | GET | `/iam/v3/admin/roles/{roleId}` | Retrieves a role by ID |
| `roles` | `get` | admin | v4 | GET | `/iam/v4/admin/roles/{roleId}` | Retrieves a role by ID |
| `roles` | `get` | public | v3 | GET | `/iam/v3/public/roles/{roleId}` | Retrieves a non-admin role by ID |
| `roles` | `get-admin-status` | admin | v3 | GET | `/iam/v3/admin/roles/{roleId}/admin` | Retrieves the admin status of a role |
| `roles` | `list` | admin | v3 | GET | `/iam/v3/admin/roles` | Lists available roles |
| `roles` | `list` | admin | v4 | GET | `/iam/v4/admin/roles` | Lists all roles in the namespace |
| `roles` | `list` | public | v3 | GET | `/iam/v3/public/roles` | Retrieves non-admin roles |
| `roles` | `list-assigned-users` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/roles/{roleId}/users` | Retrieves admin users assigned to a role |
| `roles` | `list-assigned-users` | admin | v4 | GET | `/iam/v4/admin/roles/{roleId}/users` | Lists users assigned to a role |
| `roles` | `list-managers` | admin | v3 | GET | `/iam/v3/admin/roles/{roleId}/managers` | Lists the managers of a role |
| `roles` | `list-members` | admin | v3 | GET | `/iam/v3/admin/roles/{roleId}/members` | Lists the members of a role |
| `roles` | `remove-admin-status` | admin | v3 | DELETE | `/iam/v3/admin/roles/{roleId}/admin` | Removes the admin status from a role |
| `roles` | `remove-permission` | admin | v3 | DELETE | `/iam/v3/admin/roles/{roleId}/permissions/{resource}/{action}` | Removes a permission from a role |
| `roles` | `remove-permissions` | admin | v3 | DELETE | `/iam/v3/admin/roles/{roleId}/permissions` | Removes permissions from a role |
| `roles` | `remove-permissions` | admin | v4 | DELETE | `/iam/v4/admin/roles/{roleId}/permissions` | Removes permissions from a role |
| `roles` | `revoke-users` | admin | v4 | DELETE | `/iam/v4/admin/roles/{roleId}/users` | Revokes a user from a role |
| `roles` | `update` | admin | v3 | PATCH | `/iam/v3/admin/roles/{roleId}` | Updates a role |
| `roles` | `update` | admin | v4 | PATCH | `/iam/v4/admin/roles/{roleId}` | Updates a role |
| `roles` | `update-admin-status` | admin | v3 | POST | `/iam/v3/admin/roles/{roleId}/admin` | Marks a role as an admin role |
| `roles` | `update-permissions` | admin | v3 | PUT | `/iam/v3/admin/roles/{roleId}/permissions` | Replaces a role's permissions |
| `roles` | `update-permissions` | admin | v4 | PUT | `/iam/v4/admin/roles/{roleId}/permissions` | Replaces all permissions assigned to a role |
| `sso` | `authenticate-with-saml` | public | v3 | POST | `/iam/v3/sso/saml/platforms/{platformId}/authenticate` | Authenticates a user via a SAML platform |
| `sso` | `login-client` | public | v3 | GET | `/iam/v3/sso/{platformId}` | Starts an SSO login for a platform client. |
| `sso` | `logout-client` | public | v3 | POST | `/iam/v3/sso/{platformId}/logout` | Logs out from an SSO platform session |
| `sso-credentials` | `create` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/sso` | Adds SSO platform credentials |
| `sso-credentials` | `delete` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/sso` | Deletes SSO platform credentials |
| `sso-credentials` | `get` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/sso` | Retrieves SSO platform credentials |
| `sso-credentials` | `list` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/platforms/sso` | Lists all SSO platform credentials |
| `sso-credentials` | `update` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/sso` | Updates SSO platform credentials |
| `tags` | `create` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/tags` | Creates an account identifier tag |
| `tags` | `delete` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/tags/{tagId}` | Deletes an account identifier tag |
| `tags` | `list` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/tags` | Queries account identifier tags |
| `tags` | `update` | admin | v3 | PUT | `/iam/v3/admin/namespaces/{namespace}/tags/{tagId}` | Updates an account identifier tag |
| `user-mfa` | `disable` | admin | v4 | DELETE | `/iam/v4/admin/namespaces/{namespace}/users/{userId}/mfa/disable` | Disables 2FA for a user |
| `user-mfa` | `disable-my-authenticator` | admin | v4 | DELETE | `/iam/v4/admin/users/me/mfa/authenticator/disable` | Disables 2FA authenticator for the current user |
| `user-mfa` | `disable-my-authenticator` | public | v4 | DELETE | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/authenticator/disable` | Disables 2FA authenticator for the current user |
| `user-mfa` | `disable-my-backup-codes` | admin | v4 | DELETE | `/iam/v4/admin/users/me/mfa/backupCode/disable` | Disables 2FA backup codes for the current user |
| `user-mfa` | `disable-my-backup-codes` | public | v4 | DELETE | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/backupCode/disable` | Disables 2FA backup codes for the current user |
| `user-mfa` | `disable-my-email` | admin | v4 | POST | `/iam/v4/admin/users/me/mfa/email/disable` | Disables 2FA email for the current user |
| `user-mfa` | `disable-my-email` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/email/disable` | Disables 2FA email for the current user |
| `user-mfa` | `enable-my-authenticator` | admin | v4 | POST | `/iam/v4/admin/users/me/mfa/authenticator/enable` | Enables 2FA authenticator for the current user |
| `user-mfa` | `enable-my-authenticator` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/authenticator/enable` | Enables 2FA authenticator for the current user |
| `user-mfa` | `enable-my-backup-codes` | admin | v4 | POST | `/iam/v4/admin/users/me/mfa/backupCodes/enable` | Enables 2FA backup codes for the current user |
| `user-mfa` | `enable-my-backup-codes` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/backupCodes/enable` | Enables 2FA backup codes for the current user |
| `user-mfa` | `enable-my-email` | admin | v4 | POST | `/iam/v4/admin/users/me/mfa/email/enable` | Enables 2FA email for the current user |
| `user-mfa` | `enable-my-email` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/email/enable` | Enables 2FA email for the current user |
| `user-mfa` | `generate-my-authenticator-key` | admin | v4 | POST | `/iam/v4/admin/users/me/mfa/authenticator/key` | Generates a secret key for a 3rd-party authenticator app |
| `user-mfa` | `generate-my-authenticator-key` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/authenticator/key` | Generates a secret key for a 3rd-party authenticator app |
| `user-mfa` | `generate-my-backup-codes` | admin | v4 | POST | `/iam/v4/admin/users/me/mfa/backupCodes` | Generates backup codes for the current user |
| `user-mfa` | `generate-my-backup-codes` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/backupCodes` | Generates new backup codes for the current user |
| `user-mfa` | `get-my-status` | admin | v4 | GET | `/iam/v4/admin/users/me/mfa/status` | Retrieves the current admin user's 2FA status |
| `user-mfa` | `get-my-status` | public | v4 | GET | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/status` | Retrieves the current user's 2FA status |
| `user-mfa` | `list-my-backup-codes` | admin | v4 | GET | `/iam/v4/admin/users/me/mfa/backupCodes` | Retrieves backup codes and sends them to the registered email |
| `user-mfa` | `list-my-backup-codes` | public | v4 | GET | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/backupCodes` | Retrieves existing backup codes and sends them to the registered email |
| `user-mfa` | `list-my-enabled` | admin | v4 | GET | `/iam/v4/admin/users/me/mfa/factor` | Lists 2FA factors enabled for the current user |
| `user-mfa` | `list-my-enabled` | public | v4 | GET | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/factor` | Lists 2FA factors enabled for the current user |
| `user-mfa` | `send-my-mfa-email` | admin | v4 | POST | `/iam/v4/admin/users/me/mfa/email/code` | Sends a one-time code to the registered email for MFA |
| `user-mfa` | `send-my-mfa-email` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/email/code` | Sends a one-time code to the registered email for MFA |
| `user-mfa` | `set-default` | admin | v4 | POST | `/iam/v4/admin/users/me/mfa/factor` | Sets the default 2FA factor for the current user |
| `user-mfa` | `set-my-default` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/factor` | Sets the default 2FA factor for the current user |
| `user-mfa` | `verify-my-challenge` | admin | v4 | POST | `/iam/v4/admin/users/me/mfa/challenge/verify` | Verifies a user MFA code and issues an MFA token |
| `user-mfa` | `verify-my-challenge` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/challenge/verify` | Verifies a user MFA code and issues an MFA token |
| `users` | `add-permission` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/permissions` | Adds permissions to the user |
| `users` | `assign-role` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/roles/{roleId}` | Assigns a role to a user |
| `users` | `assign-role` | admin | v4 | POST | `/iam/v4/admin/namespaces/{namespace}/users/{userId}/roles` | Adds roles to a user's current role assignments. |
| `users` | `ban` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/bans` | Bans a user |
| `users` | `bulk-check-valid` | admin | v4 | POST | `/iam/v4/admin/namespaces/{namespace}/users/bulk/validate` | Checks whether user IDs are valid |
| `users` | `bulk-get` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/bulk` | Retrieves users by user ID |
| `users` | `bulk-get-bans` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/bans` | Retrieves bans for a list of users |
| `users` | `bulk-get-by-email` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/search/bulk` | Retrieves users by email address in bulk |
| `users` | `bulk-get-platforms` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/bulk/platforms` | Retrieves platform information for a list of users |
| `users` | `bulk-remove-permissions` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/permissions` | Deletes user permissions |
| `users` | `bulk-update` | admin | v3 | PUT | `/iam/v3/admin/namespaces/{namespace}/users` | Updates users in bulk |
| `users` | `bulk-update-account-type` | admin | v4 | PATCH | `/iam/v4/admin/namespaces/{namespace}/users/bulk/accountType` | Updates user account types in bulk. |
| `users` | `check-availability` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/users/availability` | Checks whether a user account field value is available |
| `users` | `create` | admin | v4 | POST | `/iam/v4/admin/namespaces/{namespace}/users` | Creates a new user account |
| `users` | `create` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users` | Creates a new user account |
| `users` | `create` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users` | Creates a new user account |
| `users` | `create-from-invitation` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/invite/{invitationId}` | Creates a user account from an invitation |
| `users` | `create-from-invitation` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users/invite/{invitationId}` | Creates a user account from an invitation |
| `users` | `create-from-publisher` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/platforms/justice/{targetNamespace}` | Creates a Justice user from a publisher user |
| `users` | `create-from-publisher` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/me/platforms/justice/{targetNamespace}` | Creates a game namespace user from a publisher account |
| `users` | `create-test` | admin | v4 | POST | `/iam/v4/admin/namespaces/{namespace}/test_users` | Creates test users without sending verification emails |
| `users` | `create-test` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/test_users` | Creates a test user without sending a verification email |
| `users` | `delete` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/information` | Deletes a user's information |
| `users` | `delete-platform-linking-restriction` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/platforms/{platformId}/link/restrictions` | Removes a user's platform linking restriction |
| `users` | `force-link-platform` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/me/platforms/{platformId}/force` | Links the current user's account to a platform, overriding any existing link. |
| `users` | `force-link-platform-with-progression` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/{userId}/platforms/linkWithProgression` | Links a platform account and transfers game progression, overriding any existing link. |
| `users` | `force-verify` | admin | v3 | PUT | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/verify` | Verifies a user account |
| `users` | `get` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}` | Retrieves a user by user ID |
| `users` | `get` | public | v4 | GET | `/iam/v4/public/namespaces/{namespace}/users/{userId}` | Retrieves public information for a user |
| `users` | `get-ban-summary` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/bans/summary` | Retrieves a summary of ban records for a user |
| `users` | `get-by-email` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users` | Retrieves a user by email address |
| `users` | `get-by-platform-user-id` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/users/{platformUserId}` | Retrieves a user by platform user ID |
| `users` | `get-by-platform-user-id` | public | v4 | GET | `/iam/v4/public/namespaces/{namespace}/platforms/{platformId}/users/{platformUserId}` | Retrieves a user by platform user ID |
| `users` | `get-deletion-status` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/deletion/status` | Retrieves deletion status for a user |
| `users` | `get-information` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/users/{userId}/information` | Retrieves a user's profile and linked platform accounts |
| `users` | `get-invitation` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/users/invite/{invitationId}` | Retrieves a user invitation |
| `users` | `get-invitation-history` | admin | v4 | GET | `/iam/v4/admin/namespaces/{namespace}/invitationHistories` | Retrieves invitation history for a specific namespace |
| `users` | `get-linking-status` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/requests/{requestId}/async/status` | Retrieves the async account linking progress status |
| `users` | `get-login-history` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/logins/histories` | Retrieves login history for a user |
| `users` | `get-login-history` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/users/{userId}/logins/histories` | Retrieves login history for a user |
| `users` | `get-mapping` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/platforms/justice/{targetNamespace}` | Retrieves the namespace user ID mapping for a user |
| `users` | `get-mfa-status` | admin | v4 | GET | `/iam/v4/admin/namespaces/{namespace}/users/{userId}/mfa/status` | Retrieves a user's 2FA status |
| `users` | `get-my` | admin | v3 | GET | `/iam/v3/admin/users/me` | Retrieves the current user account |
| `users` | `get-my` | public | v3 | GET | `/iam/v3/public/users/me` | Retrieves the current user's profile |
| `users` | `get-my-link-redirection` | public | v3 | GET | `/iam/v3/public/users/me/link/redirection` | Retrieves the redirect URI after account linking |
| `users` | `get-my-linking-conflict` | public | v3 | GET | `/iam/v3/public/users/me/headless/link/conflict` | Retrieves linking conflicts when linking a headless account to a full account |
| `users` | `get-my-profile-update-policy` | public | v3 | GET | `/iam/v3/public/users/me/profileStatus` | Retrieves profile update status for the current user |
| `users` | `get-openid-info` | public | v3 | GET | `/iam/v3/public/users/userinfo` | Retrieves the current user's OpenID Connect user info |
| `users` | `get-platform-account-metadata` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/platforms/{platformId}/metadata` | Retrieves platform account metadata for a user |
| `users` | `get-platform-link-status` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/platforms/{platformId}/linkStatus` | Checks the link status of a third-party platform token |
| `users` | `get-platform-linking-history` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/platforms/link/histories` | Lists platform link history for a user |
| `users` | `get-platform-web-link` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/users/me/platforms/{platformId}/web/link` | Generates a third-party platform login URL for web account linking |
| `users` | `get-publisher` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/users/{userId}/publisher` | Retrieves the publisher account for a user |
| `users` | `get-state` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/state` | Retrieves the state of a user account |
| `users` | `get-user-invitation-history` | admin | v4 | GET | `/iam/v4/admin/namespaces/{namespace}/invitationHistories/users` | Retrieves user invitation history for a specific namespace |
| `users` | `handle-web-auth-callback` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/users/me/platforms/{platformId}/web/link/establish` | Creates the third-party platform account link after the authorization redirect. |
| `users` | `invite` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/invite` | Invites a user to the namespace |
| `users` | `invite` | admin | v4 | POST | `/iam/v4/admin/users/invite` | Invites a user and assigns a role |
| `users` | `invite-admin` | public | v4 | POST | `/iam/v4/public/users/invite` | Invites a game studio admin user to a new namespace |
| `users` | `link-my-headless-account` | public | v3 | POST | `/iam/v3/public/users/me/headless/linkWithProgression` | Links a headless account to the current full account |
| `users` | `link-platform-account` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/platforms/{platformId}/link` | Links a user's account to a platform |
| `users` | `link-platform-account` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/me/platforms/{platformId}` | Links the current user's account to a third-party platform |
| `users` | `link-platform-account-forced` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/platforms/link` | Links a platform account to a user |
| `users` | `list-accelbyte-accounts` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/platforms/justice` | Lists Justice platform accounts for a user |
| `users` | `list-accelbyte-accounts` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/users/{userId}/platforms/justice` | Retrieves Justice platform accounts for a user |
| `users` | `list-admins` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/admins` | Lists admin users in a namespace |
| `users` | `list-bans` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/bans` | Retrieves ban records for a user |
| `users` | `list-bans` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/users/{userId}/bans` | Retrieves ban records for a user |
| `users` | `list-by-cursor` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/cursor` | Retrieves users using cursor-based pagination |
| `users` | `list-by-platform-id` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/platforms/{platformId}/users` | Lists user IDs by platform user ID |
| `users` | `list-by-platform-id` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/platforms/{platformId}/users` | Lists AccelByte user IDs by platform user IDs |
| `users` | `list-distinct-active-platform-accounts` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/distinctPlatforms` | Retrieves distinct platform accounts linked to a user |
| `users` | `list-distinct-active-platform-accounts` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/users/{userId}/distinctPlatforms` | Retrieves distinct platform accounts linked to a user |
| `users` | `list-distinct-platform-accounts` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/platforms/distinct` | Lists distinct third-party platforms linked to a user |
| `users` | `list-invitation-histories` | admin | v4 | GET | `/iam/v4/admin/invitationHistories` | Lists invitation histories for studio namespaces |
| `users` | `list-linked-platforms` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/platforms` | Retrieves users' basic info and linked third-party platform info |
| `users` | `list-platform-accounts` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/platforms` | Lists platform accounts linked to a user |
| `users` | `list-platform-accounts` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/users/{userId}/platforms` | Retrieves platform accounts linked to a user |
| `users` | `list-roles` | admin | v4 | GET | `/iam/v4/admin/namespaces/{namespace}/users/{userId}/roles` | Lists roles assigned to a user |
| `users` | `list-users-with-accelbyte-account` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/platforms/justice` | Lists users with Justice platform accounts |
| `users` | `list-verification-codes` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/codes` | Retrieves active verification codes for a user |
| `users` | `remove-permission` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/permissions/{resource}/{action}` | Deletes a permission from a user |
| `users` | `request-password-reset` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/forgot` | Requests a password reset code |
| `users` | `request-password-reset-global` | public | v3 | POST | `/iam/v3/public/users/forgot` | Requests a password reset code |
| `users` | `reset-password` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/reset` | Resets a user's password |
| `users` | `resolve-web-auth-redirect` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/me/platforms/{platformId}/web/link/process` | Processes the third-party web link callback and returns link status |
| `users` | `revoke-role` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/roles/{roleId}` | Removes a role from a user |
| `users` | `revoke-role` | admin | v4 | DELETE | `/iam/v4/admin/namespaces/{namespace}/users/{userId}/roles` | Removes roles from a user |
| `users` | `revoke-roles` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/roles` | Removes roles from a user |
| `users` | `revoke-trusted-device` | public | v4 | DELETE | `/iam/v4/public/namespaces/{namespace}/users/me/mfa/device` | Removes the trusted device for the current user |
| `users` | `search` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/search` | Searches users in the namespace |
| `users` | `search` | public | v3 | GET | `/iam/v3/public/namespaces/{namespace}/users` | Searches for users by display name, username, or third-party display name |
| `users` | `search-platform-linking-history` | admin | v3 | GET | `/iam/v3/admin/namespaces/{namespace}/users/linkhistories` | Searches link history for a platform user ID |
| `users` | `send-my-verification-code` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/me/code/request` | Sends a verification code to the current user's email address |
| `users` | `send-verification-code` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/code/request` | Sends a verification code to a user |
| `users` | `send-verification-code` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/code/request` | Sends a verification code to a new account's email address |
| `users` | `send-verification-code-forward` | public | v3 | POST | `/iam/v3/public/users/me/code/request/forward` | Sends a verification code using an upgrade token |
| `users` | `send-verification-link` | public | v3 | POST | `/iam/v3/public/users/me/verify_link/request` | Sends a verification link to the current user's email address |
| `users` | `set-my` | public | v3 | PUT | `/iam/v3/public/namespaces/{namespace}/users/me` | Updates the current user's profile |
| `users` | `set-permissions` | admin | v3 | PUT | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/permissions` | Replaces a user's permissions |
| `users` | `set-roles` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/roles` | Replaces the roles assigned to a user |
| `users` | `set-roles` | admin | v4 | PUT | `/iam/v4/admin/namespaces/{namespace}/users/{userId}/roles` | Replaces all roles assigned to a user |
| `users` | `start-upgrade-my` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/me/headless/verify` | Upgrades a headless account to a full account with email |
| `users` | `start-upgrade-my` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users/me/headless/verify` | Upgrades a headless account to a full account |
| `users` | `unlink-platform-all` | admin | v3 | DELETE | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/platforms/{platformId}/all` | Unlinks a user's account from a platform across all namespaces |
| `users` | `unlink-platform-all` | public | v3 | DELETE | `/iam/v3/public/namespaces/{namespace}/users/me/platforms/{platformId}/all` | Unlinks the current user's account from a platform across all namespaces |
| `users` | `update` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/users/{userId}` | Updates a user |
| `users` | `update` | admin | v4 | PUT | `/iam/v4/admin/namespaces/{namespace}/users/{userId}` | Updates a user's profile |
| `users` | `update-ban` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/bans/{banId}` | Enables or disables a ban for a user |
| `users` | `update-deletion-status` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/deletion/status` | Updates deletion status for a user |
| `users` | `update-email` | admin | v4 | PUT | `/iam/v4/admin/namespaces/{namespace}/users/{userId}/email` | Updates a user's email address |
| `users` | `update-my` | admin | v4 | PATCH | `/iam/v4/admin/users/me` | Updates the current admin user's profile |
| `users` | `update-my` | public | v3 | PATCH | `/iam/v3/public/namespaces/{namespace}/users/me` | Updates the current user's profile |
| `users` | `update-my` | public | v4 | PATCH | `/iam/v4/public/namespaces/{namespace}/users/me` | Updates the current user's profile |
| `users` | `update-my-email` | public | v4 | PUT | `/iam/v4/public/namespaces/{namespace}/users/me/email` | Updates the current user's email address |
| `users` | `update-my-password` | public | v3 | PUT | `/iam/v3/public/namespaces/{namespace}/users/me/password` | Updates the current user's password |
| `users` | `update-password` | admin | v3 | PUT | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/password` | Updates a user's password |
| `users` | `update-status` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/status` | Enables or disables a user account |
| `users` | `update-trusted-identity` | admin | v3 | PATCH | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/trustly/identity` | Updates a user's identity without email notification |
| `users` | `upgrade-and-validate` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/headless/code/verify` | Verifies or consumes a verification code |
| `users` | `upgrade-and-validate` | public | v4 | POST | `/iam/v4/public/namespaces/{namespace}/users/me/headless/code/verify` | Upgrades a headless account and verifies the email address |
| `users` | `upgrade-and-validate-forward` | public | v4 | POST | `/iam/v4/public/users/me/headless/code/verify/forward` | Upgrades a headless account and verifies the email address with redirect |
| `users` | `upgrade-and-validate-my` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/me/headless/code/verify` | Verifies or consumes a code to upgrade a headless account |
| `users` | `validate-code` | admin | v3 | POST | `/iam/v3/admin/namespaces/{namespace}/users/{userId}/code/verify` | Verifies or consumes a verification code for a user |
| `users` | `validate-code` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/me/code/verify` | Validates or consumes a verification code for the current user |
| `users` | `validate-input` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/input/validation` | Validates user input against configured validation rules |
| `users` | `validate-password` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/{userId}/validate` | Validates a user's password |
| `users` | `validate-registration-code` | public | v3 | POST | `/iam/v3/public/namespaces/{namespace}/users/code/verify` | Verifies the registration code |
| `users` | `verify-email` | public | v3 | GET | `/iam/v3/public/users/verify_link/verify` | Verifies a user's email using a verification link code |

## inventory

- Spec name: `inventory`
- Resources: 7
- Operations: 43

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `chaining` | `create` | admin | v1 | POST | `/inventory/v1/admin/namespaces/{namespace}/chainingOperations` | Creates a chaining operation |
| `integrations` | `create` | admin | v1 | POST | `/inventory/v1/admin/namespaces/{namespace}/integrationConfigurations` | Creates an integration configuration |
| `integrations` | `list` | admin | v1 | GET | `/inventory/v1/admin/namespaces/{namespace}/integrationConfigurations` | Lists integration configurations |
| `integrations` | `update` | admin | v1 | PUT | `/inventory/v1/admin/namespaces/{namespace}/integrationConfigurations/{integrationConfigurationId}` | Updates an integration configuration |
| `integrations` | `update-status` | admin | v1 | PUT | `/inventory/v1/admin/namespaces/{namespace}/integrationConfigurations/{integrationConfigurationId}/status` | Updates integration configuration status |
| `inventories` | `check-purchasable` | admin | v1 | POST | `/inventory/v1/admin/namespaces/{namespace}/users/{userId}/purchaseable` | Checks user inventory capacity for an e-commerce order |
| `inventories` | `create` | admin | v1 | POST | `/inventory/v1/admin/namespaces/{namespace}/inventories` | Creates an inventory |
| `inventories` | `delete` | admin | v1 | DELETE | `/inventory/v1/admin/namespaces/{namespace}/inventories/{inventoryId}` | Deletes an inventory |
| `inventories` | `get` | admin | v1 | GET | `/inventory/v1/admin/namespaces/{namespace}/inventories/{inventoryId}` | Retrieves an inventory |
| `inventories` | `list` | admin | v1 | GET | `/inventory/v1/admin/namespaces/{namespace}/inventories` | Lists inventories |
| `inventories` | `list` | public | v1 | GET | `/inventory/v1/public/namespaces/{namespace}/users/me/inventories` | Lists the current user's inventories |
| `inventories` | `update` | admin | v1 | PUT | `/inventory/v1/admin/namespaces/{namespace}/inventories/{inventoryId}` | Updates an inventory |
| `inventories` | `update-user` | admin | v1 | PUT | `/inventory/v1/admin/namespaces/{namespace}/users/{userId}/inventoryConfigurations/{inventoryConfigurationCode}/inventories` | Updates user inventories by inventory configuration code |
| `inventory-definitions` | `create` | admin | v1 | POST | `/inventory/v1/admin/namespaces/{namespace}/inventoryConfigurations` | Creates an inventory configuration |
| `inventory-definitions` | `delete` | admin | v1 | DELETE | `/inventory/v1/admin/namespaces/{namespace}/inventoryConfigurations/{inventoryConfigurationId}` | Deletes an inventory configuration |
| `inventory-definitions` | `get` | admin | v1 | GET | `/inventory/v1/admin/namespaces/{namespace}/inventoryConfigurations/{inventoryConfigurationId}` | Retrieves an inventory configuration |
| `inventory-definitions` | `list` | admin | v1 | GET | `/inventory/v1/admin/namespaces/{namespace}/inventoryConfigurations` | Lists inventory configurations |
| `inventory-definitions` | `list` | public | v1 | GET | `/inventory/v1/public/namespaces/{namespace}/inventoryConfigurations` | Lists inventory configurations |
| `inventory-definitions` | `update` | admin | v1 | PUT | `/inventory/v1/admin/namespaces/{namespace}/inventoryConfigurations/{inventoryConfigurationId}` | Updates an inventory configuration |
| `item-types` | `create` | admin | v1 | POST | `/inventory/v1/admin/namespaces/{namespace}/itemtypes` | Creates an item type |
| `item-types` | `delete` | admin | v1 | DELETE | `/inventory/v1/admin/namespaces/{namespace}/itemtypes/{itemTypeName}` | Deletes an item type |
| `item-types` | `list` | admin | v1 | GET | `/inventory/v1/admin/namespaces/{namespace}/itemtypes` | Lists item types |
| `item-types` | `list` | public | v1 | GET | `/inventory/v1/public/namespaces/{namespace}/itemtypes` | Lists item types |
| `items` | `add` | admin | v1 | POST | `/inventory/v1/admin/namespaces/{namespace}/users/{userId}/inventories/{inventoryId}/items` | Saves items to a specific inventory |
| `items` | `add-user` | admin | v1 | POST | `/inventory/v1/admin/namespaces/{namespace}/users/{userId}/items` | Saves an item to the user's inventory |
| `items` | `bulk-add` | admin | v1 | POST | `/inventory/v1/admin/namespaces/{namespace}/users/{userId}/inventories/{inventoryId}/items/bulk` | Saves items to a specific inventory |
| `items` | `bulk-add-user` | admin | v1 | POST | `/inventory/v1/admin/namespaces/{namespace}/users/{userId}/items/bulk` | Saves items to the user's inventory |
| `items` | `bulk-remove` | admin | v1 | DELETE | `/inventory/v1/admin/namespaces/{namespace}/users/{userId}/inventories/{inventoryId}/items` | Removes inventory items |
| `items` | `bulk-remove-my` | public | v1 | DELETE | `/inventory/v1/public/namespaces/{namespace}/users/me/inventories/{inventoryId}/items` | Removes items from the current user's inventory |
| `items` | `bulk-update` | admin | v1 | PUT | `/inventory/v1/admin/namespaces/{namespace}/users/{userId}/inventories/{inventoryId}/items` | Updates inventory items |
| `items` | `bulk-update-my` | public | v1 | PUT | `/inventory/v1/public/namespaces/{namespace}/users/me/inventories/{inventoryId}/items` | Updates items in the current user's inventory |
| `items` | `consume-my` | public | v1 | POST | `/inventory/v1/public/namespaces/{namespace}/users/me/inventories/{inventoryId}/consume` | Updates the current user's inventory by consuming an item |
| `items` | `consume-user` | admin | v1 | POST | `/inventory/v1/admin/namespaces/{namespace}/users/{userId}/inventories/{inventoryId}/consume` | Consumes an item from a user's inventory |
| `items` | `get` | admin | v1 | GET | `/inventory/v1/admin/namespaces/{namespace}/inventories/{inventoryId}/slots/{slotId}/sourceItems/{sourceItemId}` | Retrieves an inventory item |
| `items` | `get` | public | v1 | GET | `/inventory/v1/public/namespaces/{namespace}/users/me/inventories/{inventoryId}/slots/{slotId}/sourceItems/{sourceItemId}` | Gets an item from the current user's inventory |
| `items` | `list` | admin | v1 | GET | `/inventory/v1/admin/namespaces/{namespace}/inventories/{inventoryId}/items` | Lists inventory items |
| `items` | `list` | public | v1 | GET | `/inventory/v1/public/namespaces/{namespace}/users/me/inventories/{inventoryId}/items` | Lists items in the current user's inventory |
| `items` | `move-my` | public | v1 | POST | `/inventory/v1/public/namespaces/{namespace}/users/me/inventories/{inventoryId}/items/movement` | Moves items between the current user's inventories |
| `items` | `sync-entitlements` | admin | v1 | PUT | `/inventory/v1/admin/namespaces/{namespace}/users/{userId}/items/entitlements/sync` | Updates inventory from user entitlements |
| `tags` | `create` | admin | v1 | POST | `/inventory/v1/admin/namespaces/{namespace}/tags` | Creates a tag |
| `tags` | `delete` | admin | v1 | DELETE | `/inventory/v1/admin/namespaces/{namespace}/tags/{tagName}` | Deletes a tag |
| `tags` | `list` | admin | v1 | GET | `/inventory/v1/admin/namespaces/{namespace}/tags` | Lists tags |
| `tags` | `list` | public | v1 | GET | `/inventory/v1/public/namespaces/{namespace}/tags` | Lists tags |

## leaderboard

- Spec name: `leaderboard`
- Resources: 4
- Operations: 62

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `global-rankings` | `get-snapshot-url` | admin | v1 | GET | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/archived` | Retrieves signed download URLs for archived all-time leaderboard ranking data |
| `global-rankings` | `get-snapshot-url` | public | v1 | GET | `/leaderboard/v1/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/archived` | Retrieves signed download URLs for archived all-time leaderboard ranking data |
| `global-rankings` | `list-all-time` | admin | v1 | GET | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/alltime` | Retrieves all-time leaderboard ranking data |
| `global-rankings` | `list-all-time` | admin | v3 | GET | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/alltime` | Retrieves all-time leaderboard ranking data |
| `global-rankings` | `list-all-time` | public | v1 | GET | `/leaderboard/v1/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/alltime` | Retrieves all-time leaderboard ranking data |
| `global-rankings` | `list-all-time` | public | v2 | GET | `/leaderboard/v2/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/alltime` | Retrieves all-time leaderboard ranking data |
| `global-rankings` | `list-all-time` | public | v3 | GET | `/leaderboard/v3/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/alltime` | Retrieves all-time leaderboard ranking data |
| `global-rankings` | `list-cycle` | admin | v3 | GET | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/cycles/{cycleId}` | Retrieves ranking data for a leaderboard cycle |
| `global-rankings` | `list-cycle` | public | v3 | GET | `/leaderboard/v3/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/cycles/{cycleId}` | Retrieves ranking data for a leaderboard cycle |
| `global-rankings` | `list-month` | admin | v1 | GET | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/month` | Retrieves current-month leaderboard ranking data |
| `global-rankings` | `list-month` | public | v1 | GET | `/leaderboard/v1/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/month` | Retrieves current-month leaderboard ranking data |
| `global-rankings` | `list-season` | admin | v1 | GET | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/season` | Retrieves current-season leaderboard ranking data |
| `global-rankings` | `list-season` | public | v1 | GET | `/leaderboard/v1/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/season` | Retrieves current-season leaderboard ranking data |
| `global-rankings` | `list-today` | admin | v1 | GET | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/today` | Retrieves today's leaderboard ranking data |
| `global-rankings` | `list-today` | public | v1 | GET | `/leaderboard/v1/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/today` | Retrieves today's leaderboard ranking data |
| `global-rankings` | `list-week` | admin | v1 | GET | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/week` | Retrieves current-week leaderboard ranking data |
| `global-rankings` | `list-week` | public | v1 | GET | `/leaderboard/v1/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/week` | Retrieves current-week leaderboard ranking data |
| `global-rankings` | `reset` | admin | v1 | DELETE | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/reset` | Deletes all user rankings for a leaderboard |
| `global-rankings` | `reset` | admin | v3 | DELETE | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/reset` | Deletes all user rankings for a leaderboard |
| `global-rankings` | `reset-cycle` | admin | v3 | DELETE | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/cycles/{cycleId}/reset` | Deletes all user rankings for a leaderboard cycle |
| `global-rankings` | `snapshot` | admin | v1 | POST | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/archived` | Creates ranking data archives for the specified leaderboards |
| `leaderboards` | `bulk-delete` | admin | v1 | POST | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/delete` | Deletes leaderboard configurations in bulk |
| `leaderboards` | `bulk-delete` | admin | v3 | POST | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/delete` | Deletes leaderboard configurations in bulk |
| `leaderboards` | `create` | admin | v1 | POST | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards` | Creates a new leaderboard configuration |
| `leaderboards` | `create` | admin | v3 | POST | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards` | Creates a new leaderboard configuration |
| `leaderboards` | `create` | public | v1 | POST | `/leaderboard/v1/public/namespaces/{namespace}/leaderboards` | Creates a new leaderboard configuration |
| `leaderboards` | `delete` | admin | v1 | DELETE | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}` | Deletes a leaderboard configuration by its code |
| `leaderboards` | `delete` | admin | v3 | DELETE | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}` | Deletes a leaderboard configuration by its code |
| `leaderboards` | `delete-permanent` | admin | v1 | DELETE | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/hard` | Deletes a leaderboard configuration and all its data permanently |
| `leaderboards` | `delete-permanent` | admin | v3 | DELETE | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/hard` | Deletes a leaderboard configuration and all its data permanently |
| `leaderboards` | `get` | admin | v1 | GET | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}` | Retrieves a leaderboard configuration by its code |
| `leaderboards` | `get` | admin | v3 | GET | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}` | Retrieves a leaderboard configuration by its code |
| `leaderboards` | `get` | public | v1 | GET | `/leaderboard/v1/public/namespaces/{namespace}/leaderboards` | Lists all leaderboard configurations in a namespace |
| `leaderboards` | `get` | public | v2 | GET | `/leaderboard/v2/public/namespaces/{namespace}/leaderboards` | Lists all leaderboard configurations in a namespace |
| `leaderboards` | `get` | public | v3 | GET | `/leaderboard/v3/public/namespaces/{namespace}/leaderboards/{leaderboardCode}` | Retrieves a leaderboard configuration by its code |
| `leaderboards` | `list` | admin | v1 | GET | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards` | Lists all leaderboard configurations in a namespace |
| `leaderboards` | `list` | admin | v3 | GET | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards` | Lists all leaderboard configurations in a namespace |
| `leaderboards` | `list` | public | v3 | GET | `/leaderboard/v3/public/namespaces/{namespace}/leaderboards` | Lists all leaderboard configurations in a namespace |
| `leaderboards` | `update` | admin | v1 | PUT | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}` | Updates a leaderboard configuration by its code |
| `leaderboards` | `update` | admin | v3 | PUT | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}` | Updates a leaderboard configuration by its code |
| `user-rankings` | `bulk-get` | public | v3 | POST | `/leaderboard/v3/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/bulk` | Retrieves rankings for a set of users in a leaderboard |
| `user-rankings` | `list` | admin | v1 | GET | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/{userId}` | Retrieves a user's ranking in a leaderboard |
| `user-rankings` | `list` | admin | v3 | GET | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/{userId}` | Retrieves a user's ranking in a leaderboard |
| `user-rankings` | `list` | public | v1 | GET | `/leaderboard/v1/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/{userId}` | Retrieves a user's ranking in a leaderboard |
| `user-rankings` | `list` | public | v3 | GET | `/leaderboard/v3/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/{userId}` | Retrieves a user's ranking in a leaderboard |
| `user-rankings` | `list-all` | admin | v1 | GET | `/leaderboard/v1/admin/namespaces/{namespace}/users/{userId}/leaderboards` | Retrieves a user's rankings across leaderboards |
| `user-rankings` | `list-all` | admin | v3 | GET | `/leaderboard/v3/admin/namespaces/{namespace}/users/{userId}/leaderboards` | Retrieves a user's rankings across leaderboards |
| `user-rankings` | `reset` | admin | v1 | DELETE | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/{userId}` | Deletes a user's ranking from a leaderboard |
| `user-rankings` | `reset` | admin | v3 | DELETE | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/{userId}` | Deletes a user's ranking from a leaderboard |
| `user-rankings` | `reset` | public | v1 | DELETE | `/leaderboard/v1/public/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/{userId}` | Deletes a user's ranking from a leaderboard |
| `user-rankings` | `reset-all` | admin | v1 | DELETE | `/leaderboard/v1/admin/namespaces/{namespace}/users/{userId}` | Deletes a user's rankings across all leaderboards |
| `user-rankings` | `reset-all` | admin | v3 | DELETE | `/leaderboard/v3/admin/namespaces/{namespace}/users/{userId}` | Deletes a user's rankings across all leaderboards |
| `user-rankings` | `reset-cycle` | admin | v3 | DELETE | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/cycles/{cycleId}/users/{userId}` | Deletes a user's ranking for a leaderboard cycle |
| `user-rankings` | `update-points` | admin | v1 | PUT | `/leaderboard/v1/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/{userId}` | Updates a user's point total in a leaderboard |
| `visibility` | `get-user-status` | admin | v2 | GET | `/leaderboard/v2/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/{userId}/visibility` | Retrieves a user's visibility status on a leaderboard |
| `visibility` | `get-user-status` | admin | v3 | GET | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/{userId}/visibility` | Retrieves a user's visibility status on a leaderboard |
| `visibility` | `list-hidden` | admin | v2 | GET | `/leaderboard/v2/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/hidden` | Retrieves hidden users on a leaderboard |
| `visibility` | `list-hidden` | admin | v3 | GET | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/hidden` | Retrieves hidden users on a leaderboard |
| `visibility` | `set-user-status` | admin | v2 | PUT | `/leaderboard/v2/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/{userId}/visibility` | Sets a user's visibility status on a specific leaderboard |
| `visibility` | `set-user-status` | admin | v3 | PUT | `/leaderboard/v3/admin/namespaces/{namespace}/leaderboards/{leaderboardCode}/users/{userId}/visibility` | Sets a user's visibility status on a specific leaderboard |
| `visibility` | `set-user-status-all` | admin | v2 | PUT | `/leaderboard/v2/admin/namespaces/{namespace}/users/{userId}/visibility` | Sets a user's visibility status across all leaderboards |
| `visibility` | `set-user-status-all` | admin | v3 | PUT | `/leaderboard/v3/admin/namespaces/{namespace}/users/{userId}/visibility` | Sets a user's visibility status across all leaderboards |

## legal

- Spec name: `legal`
- Resources: 8
- Operations: 69

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `agreements` | `accept` | public | v1 | POST | `/agreement/public/agreements/localized-policy-versions/{localizedPolicyVersionId}` | Accepts a localized policy version |
| `agreements` | `bulk-accept` | public | v1 | POST | `/agreement/public/agreements/policies` | Accepts policy versions in bulk |
| `agreements` | `bulk-accept-on-behalf` | admin | v1 | POST | `/agreement/admin/namespaces/{namespace}/users/{userId}/agreements/policies` | Accepts a set of policy versions on behalf of a user |
| `agreements` | `bulk-get-accepted` | admin | v1 | POST | `/agreement/admin/namespaces/{namespace}/agreements` | Retrieves accepted legal agreements for a set of users |
| `agreements` | `generate-exported-csv-download-url` | admin | v1 | GET | `/agreement/admin/namespaces/{namespace}/agreements/policy-versions/users/export-csv/download` | Downloads an exported CSV of users' accepted agreements |
| `agreements` | `list-accepted` | admin | v1 | GET | `/agreement/admin/namespaces/{namespace}/agreements/policies/users/{userId}` | Retrieves accepted legal agreements for a user |
| `agreements` | `list-accepted-deprecated` | admin | v1 | GET | `/agreement/admin/agreements/policies/users/{userId}` | Retrieves accepted legal agreements for a user |
| `agreements` | `list-my-accepted` | public | v1 | GET | `/agreement/public/agreements/policies` | Retrieves the current user's accepted legal agreements |
| `agreements` | `list-users-by-version` | admin | v1 | GET | `/agreement/admin/namespaces/{namespace}/agreements/policy-versions/users` | Retrieves users who have accepted a specific policy version |
| `agreements` | `list-users-by-version-deprecated` | admin | v1 | GET | `/agreement/admin/agreements/policy-versions/users` | Retrieves users who have accepted a specific policy version |
| `agreements` | `start-csv-export` | admin | v1 | POST | `/agreement/admin/namespaces/{namespace}/agreements/policy-versions/users/export-csv/initiate` | Starts an export of users' accepted agreements to CSV |
| `agreements` | `update-consent` | admin | v1 | PATCH | `/agreement/admin/agreements/localized-policy-versions/preferences/namespaces/{namespace}/userId/{userId}` | Updates a user's preference consent settings |
| `agreements` | `update-consent` | public | v1 | PATCH | `/agreement/public/agreements/localized-policy-versions/preferences` | Accepts or revokes marketing preference consent |
| `base-policies` | `create` | admin | v1 | POST | `/agreement/admin/namespaces/{namespace}/base-policies` | Creates a base legal policy |
| `base-policies` | `create-child` | admin | v1 | POST | `/agreement/admin/namespaces/{namespace}/base-policies/{basePolicyId}/policies` | Creates a policy under a base legal policy |
| `base-policies` | `create-deprecated` | admin | v1 | POST | `/agreement/admin/base-policies` | Creates a base legal policy |
| `base-policies` | `delete` | admin | v1 | DELETE | `/agreement/admin/namespaces/{namespace}/base-policies/{basePolicyId}` | Deletes a base legal policy |
| `base-policies` | `get` | admin | v1 | GET | `/agreement/admin/namespaces/{namespace}/base-policies/{basePolicyId}` | Retrieves a base legal policy |
| `base-policies` | `get-child` | admin | v1 | GET | `/agreement/admin/namespaces/{namespace}/base-policies/{basePolicyId}/countries/{countryCode}` | Retrieves a base legal policy for a specific country |
| `base-policies` | `get-child-deprecated` | admin | v1 | GET | `/agreement/admin/base-policies/{basePolicyId}/countries/{countryCode}` | Retrieves a base legal policy for a specific country |
| `base-policies` | `get-deprecated` | admin | v1 | GET | `/agreement/admin/base-policies/{basePolicyId}` | Retrieves a base legal policy |
| `base-policies` | `list` | admin | v1 | GET | `/agreement/admin/namespaces/{namespace}/base-policies` | Retrieves base legal policies in a namespace |
| `base-policies` | `list-children` | admin | v1 | GET | `/agreement/admin/namespaces/{namespace}/base-policies/{basePolicyId}/policies` | Retrieves all policies under a base legal policy |
| `base-policies` | `list-deprecated` | admin | v1 | GET | `/agreement/admin/base-policies` | Retrieves all base legal policies |
| `base-policies` | `list-types` | admin | v1 | GET | `/agreement/admin/namespaces/{namespace}/policy-types` | Retrieves all supported policy types |
| `base-policies` | `update` | admin | v1 | PATCH | `/agreement/admin/namespaces/{namespace}/base-policies/{basePolicyId}` | Updates a base legal policy |
| `base-policies` | `update-deprecated` | admin | v1 | PATCH | `/agreement/admin/base-policies/{basePolicyId}` | Updates a base legal policy |
| `eligibilities` | `get` | public | v1 | GET | `/agreement/public/eligibilities/namespaces/{namespace}/countries/{countryCode}/clients/{clientId}/users/{userId}` | Checks legal eligibility for a specific user |
| `eligibilities` | `get-on-behalf` | admin | v1 | GET | `/agreement/admin/namespaces/{namespace}/users/{userId}/eligibilities` | Checks a user's legal eligibility against active policies |
| `eligibilities` | `list` | public | v1 | GET | `/agreement/public/eligibilities/namespaces/{namespace}` | Checks the current user's legal eligibility |
| `localized-versions` | `create` | admin | v1 | POST | `/agreement/admin/namespaces/{namespace}/localized-policy-versions/versions/{policyVersionId}` | Creates a localized version of a country-specific policy |
| `localized-versions` | `create-deprecated` | admin | v1 | POST | `/agreement/admin/localized-policy-versions/versions/{policyVersionId}` | Creates a localized version of a country-specific policy |
| `localized-versions` | `delete` | admin | v1 | DELETE | `/agreement/admin/namespaces/{namespace}/localized-policy-versions/versions/{localizedPolicyVersionId}` | Deletes a localized policy version |
| `localized-versions` | `generate-upload-url` | admin | v1 | POST | `/agreement/admin/namespaces/{namespace}/localized-policy-versions/{localizedPolicyVersionId}/attachments` | Requests a pre-signed URL for uploading a policy attachment |
| `localized-versions` | `generate-upload-url-deprecated` | admin | v1 | POST | `/agreement/admin/localized-policy-versions/{localizedPolicyVersionId}/attachments` | Requests a pre-signed URL for uploading a policy attachment |
| `localized-versions` | `get` | admin | v1 | GET | `/agreement/admin/namespaces/{namespace}/localized-policy-versions/{localizedPolicyVersionId}` | Retrieves a localized version of a country-specific policy |
| `localized-versions` | `get` | public | v1 | GET | `/agreement/public/namespaces/{namespace}/localized-policy-versions/{localizedPolicyVersionId}` | Retrieves a localized policy version for a namespace |
| `localized-versions` | `get-deprecated` | admin | v1 | GET | `/agreement/admin/localized-policy-versions/{localizedPolicyVersionId}` | Retrieves a localized version of a country-specific policy |
| `localized-versions` | `get-global` | public | v1 | GET | `/agreement/public/localized-policy-versions/{localizedPolicyVersionId}` | Retrieves a localized policy version |
| `localized-versions` | `list` | admin | v1 | GET | `/agreement/admin/namespaces/{namespace}/localized-policy-versions/versions/{policyVersionId}` | Retrieves localized versions of a country-specific policy |
| `localized-versions` | `list-deprecated` | admin | v1 | GET | `/agreement/admin/localized-policy-versions/versions/{policyVersionId}` | Retrieves localized versions of a country-specific policy |
| `localized-versions` | `set-default` | admin | v1 | PATCH | `/agreement/admin/namespaces/{namespace}/localized-policy-versions/{localizedPolicyVersionId}/default` | Sets a localized policy version as the default |
| `localized-versions` | `set-default-deprecated` | admin | v1 | PATCH | `/agreement/admin/localized-policy-versions/{localizedPolicyVersionId}/default` | Sets a localized policy version as the default |
| `localized-versions` | `update` | admin | v1 | PUT | `/agreement/admin/namespaces/{namespace}/localized-policy-versions/{localizedPolicyVersionId}` | Updates a localized version of a country-specific policy |
| `localized-versions` | `update-deprecated` | admin | v1 | PUT | `/agreement/admin/localized-policy-versions/{localizedPolicyVersionId}` | Updates a localized version of a country-specific policy |
| `policies` | `delete` | admin | v1 | DELETE | `/agreement/admin/namespaces/{namespace}/policies/{policyId}` | Deletes a country-specific policy |
| `policies` | `list` | admin | v1 | GET | `/agreement/admin/policies/countries/{countryCode}` | Retrieves active policies for a country |
| `policies` | `list` | public | v1 | GET | `/agreement/public/policies/namespaces/{namespace}/countries/{countryCode}` | Retrieves the latest active policies for a namespace and country |
| `policies` | `list` | public | v2 | GET | `/agreement/v2/public/policies/namespaces/{namespace}/countries/{countryCode}` | Retrieves the latest active policies for a namespace and country (v2) |
| `policies` | `list-all` | public | v1 | GET | `/agreement/public/policies/countries/{countryCode}` | Retrieves the latest active policies for a country |
| `policies` | `list-countries` | public | v1 | GET | `/agreement/public/policies/countries/list` | Retrieves countries that have active legal policies |
| `policies` | `list-my-country` | public | v1 | GET | `/agreement/public/policies/namespaces/{namespace}` | Retrieves the latest active policies for a namespace and country |
| `policies` | `list-types` | admin | v1 | GET | `/agreement/admin/policy-types` | Retrieves all supported policy types |
| `policies` | `set-default` | admin | v1 | PATCH | `/agreement/admin/namespaces/{namespace}/policies/{policyId}/default` | Sets a policy as the default |
| `policies` | `set-default-global` | admin | v1 | PATCH | `/agreement/admin/policies/{policyId}/default` | Sets a policy as the default |
| `policies` | `update` | admin | v1 | PATCH | `/agreement/admin/namespaces/{namespace}/policies/{policyId}` | Updates a country-specific policy |
| `policies` | `update-global` | admin | v1 | PATCH | `/agreement/admin/policies/{policyId}` | Updates a country-specific policy |
| `policy-versions` | `create` | admin | v1 | POST | `/agreement/admin/policies/{policyId}/versions` | Creates a version of a country-specific policy |
| `policy-versions` | `list-versions` | admin | v1 | GET | `/agreement/admin/policies/{policyId}/versions` | Retrieves versions of a country-specific policy |
| `utility` | `check-readiness` | public | v1 | GET | `/agreement/public/readiness` | Checks whether legal data is ready |
| `utility` | `get-status` | admin | v1 | GET | `/agreement/admin/userInfo` | Retrieves user info cache status per namespace |
| `versions` | `create` | admin | v1 | POST | `/agreement/admin/namespaces/{namespace}/policies/{policyId}/versions` | Creates a version of a country-specific policy |
| `versions` | `delete` | admin | v1 | DELETE | `/agreement/admin/namespaces/{namespace}/policies/versions/{policyVersionId}` | Deletes a version of a policy |
| `versions` | `list` | admin | v1 | GET | `/agreement/admin/namespaces/{namespace}/policies/{policyId}/versions` | Retrieves versions of a country-specific policy |
| `versions` | `publish` | admin | v1 | PATCH | `/agreement/admin/namespaces/{namespace}/policies/versions/{policyVersionId}/latest` | Publishes a country-specific policy version |
| `versions` | `publish-deprecated` | admin | v1 | PATCH | `/agreement/admin/policies/versions/{policyVersionId}/latest` | Publishes a country-specific policy version |
| `versions` | `unpublish` | admin | v1 | PATCH | `/agreement/admin/namespaces/{namespace}/policies/versions/{policyVersionId}/unpublish` | Revokes the published status of a policy version |
| `versions` | `update` | admin | v1 | PATCH | `/agreement/admin/namespaces/{namespace}/policies/versions/{policyVersionId}` | Updates a version of a policy |
| `versions` | `update-deprecated` | admin | v1 | PATCH | `/agreement/admin/policies/versions/{policyVersionId}` | Updates a version of a policy |

## lobby

- Spec name: `lobby`
- Resources: 7
- Operations: 71

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `blocks` | `symc` | public | v1 | PATCH | `/lobby/sync/namespaces/{namespace}/me/block` | Syncs the current user's blocked list with linked native first-party platform accounts |
| `config` | `export` | admin | v1 | GET | `/lobby/v1/admin/config/namespaces/{namespace}/export` | Exports the lobby configuration to a JSON file |
| `config` | `get` | admin | v1 | GET | `/lobby/v1/admin/config/namespaces/{namespace}` | Retrieves the configuration for a specified namespace |
| `config` | `get-log` | admin | v1 | GET | `/lobby/v1/admin/config/log` | Retrieves the current log configuration |
| `config` | `import` | admin | v1 | POST | `/lobby/v1/admin/config/namespaces/{namespace}/import` | Imports lobby configuration from a JSON file |
| `config` | `list` | admin | v1 | GET | `/lobby/v1/admin/config` | Retrieves configuration for all namespaces |
| `config` | `update` | admin | v1 | PUT | `/lobby/v1/admin/config/namespaces/{namespace}` | Updates the configuration for a specified namespace |
| `config` | `update-log` | admin | v1 | PATCH | `/lobby/v1/admin/config/log` | Updates the log configuration |
| `friends` | `accept-request` | public | v1 | POST | `/friends/namespaces/{namespace}/me/request/accept` | Accepts a pending incoming friend request |
| `friends` | `bulk-add` | public | v1 | POST | `/friends/namespaces/{namespace}/users/{userId}/add/bulk` | Adds users as friends in bulk without requiring confirmation |
| `friends` | `bulk-remove` | public | v1 | POST | `/friends/namespaces/{namespace}/users/{userId}/delete/bulk` | Deletes friends and removes related friend requests in bulk |
| `friends` | `cancel-request` | public | v1 | POST | `/friends/namespaces/{namespace}/me/request/cancel` | Cancels an outgoing friend request |
| `friends` | `get-status` | public | v1 | GET | `/friends/namespaces/{namespace}/me/status/{friendId}` | Retrieves the friendship status with a specified user |
| `friends` | `list` | admin | v1 | GET | `/lobby/v1/admin/friend/namespaces/{namespace}/users/{userId}` | Lists friends for a specified user |
| `friends` | `list` | public | v1 | GET | `/friends/namespaces/{namespace}/me` | Lists friends in the current user's namespace |
| `friends` | `list-friends-of-friends` | admin | v1 | GET | `/lobby/v1/admin/friend/namespaces/{namespace}/users/{userId}/of-friends` | Lists friends and friends of friends for a specified user |
| `friends` | `list-incoming-requests` | admin | v1 | GET | `/lobby/v1/admin/friend/namespaces/{namespace}/users/{userId}/incoming` | Lists incoming friend requests for a specified user |
| `friends` | `list-my-incoming-requests` | public | v1 | GET | `/friends/namespaces/{namespace}/me/incoming` | Lists incoming friend requests for the current user |
| `friends` | `list-my-incoming-requests-with-time` | public | v1 | GET | `/friends/namespaces/{namespace}/me/incoming-time` | Lists incoming friend requests with timestamp data for the current user |
| `friends` | `list-my-outgoing-requests` | public | v1 | GET | `/friends/namespaces/{namespace}/me/outgoing` | Lists outgoing friend requests sent by the current user |
| `friends` | `list-my-outgoing-requests-with-time` | public | v1 | GET | `/friends/namespaces/{namespace}/me/outgoing-time` | Lists outgoing friend requests with timestamp data for the current user |
| `friends` | `list-my-platform` | public | v1 | GET | `/friends/namespaces/{namespace}/me/platforms` | Lists friends with linked platform account data for the current user |
| `friends` | `list-outgoing-requests` | admin | v1 | GET | `/lobby/v1/admin/friend/namespaces/{namespace}/users/{userId}/outgoing` | Lists outgoing friend requests for a specified user |
| `friends` | `reject-my-request` | public | v1 | POST | `/friends/namespaces/{namespace}/me/request/reject` | Rejects an incoming friend request |
| `friends` | `send-my-request` | public | v1 | POST | `/friends/namespaces/{namespace}/me/request` | Sends a friend request to another user |
| `friends` | `sync-my` | public | v1 | PATCH | `/friends/sync/namespaces/{namespace}/me` | Syncs the current user's friend list with linked native first-party platform accounts |
| `friends` | `unfriend-my` | public | v1 | POST | `/friends/namespaces/{namespace}/me/unfriend` | Removes a user from the current user's friend list |
| `notifications` | `bulk-send` | admin | v1 | POST | `/lobby/v1/admin/notification/namespaces/{namespace}/bulkUsers/freeform/notify` | Sends a freeform notification to a list of users |
| `notifications` | `create-template` | admin | v1 | POST | `/lobby/v1/admin/notification/namespaces/{namespace}/templates` | Creates a new notification template |
| `notifications` | `create-topic` | admin | v1 | POST | `/lobby/v1/admin/notification/namespaces/{namespace}/topics` | Creates a new notification topic |
| `notifications` | `create-topic` | public | v1 | POST | `/notification/namespaces/{namespace}/topics` | Creates a new notification topic |
| `notifications` | `delete-template` | admin | v1 | DELETE | `/lobby/v1/admin/notification/namespaces/{namespace}/templates/{templateSlug}` | Deletes a notification template and all its localisations |
| `notifications` | `delete-template-localization` | admin | v1 | DELETE | `/lobby/v1/admin/notification/namespaces/{namespace}/templates/{templateSlug}/languages/{templateLanguage}` | Deletes a notification template localisation |
| `notifications` | `delete-topic` | admin | v1 | DELETE | `/lobby/v1/admin/notification/namespaces/{namespace}/topics/{topicName}` | Deletes a notification topic |
| `notifications` | `delete-topic` | public | v1 | DELETE | `/notification/namespaces/{namespace}/topics/{topic}` | Deletes a notification topic |
| `notifications` | `get-template-localization` | admin | v1 | GET | `/lobby/v1/admin/notification/namespaces/{namespace}/templates/{templateSlug}/languages/{templateLanguage}` | Retrieves a notification template localisation |
| `notifications` | `get-topic` | admin | v1 | GET | `/lobby/v1/admin/notification/namespaces/{namespace}/topics/{topicName}` | Retrieves information about a notification topic |
| `notifications` | `get-topic` | public | v1 | GET | `/notification/namespaces/{namespace}/topics/{topic}` | Retrieves information about a notification topic |
| `notifications` | `list-my` | public | v1 | GET | `/notification/namespaces/{namespace}/me` | Lists notifications for the current user |
| `notifications` | `list-my-offline` | public | v1 | GET | `/notification/namespaces/{namespace}/notification/offline/me` | Lists offline notifications for the current user |
| `notifications` | `list-template-localizations` | admin | v1 | GET | `/lobby/v1/admin/notification/namespaces/{namespace}/templates/{templateSlug}` | Lists all localisations for a notification template |
| `notifications` | `list-templates` | admin | v1 | GET | `/lobby/v1/admin/notification/namespaces/{namespace}/templates` | Lists all notification templates in the namespace |
| `notifications` | `list-topics` | admin | v1 | GET | `/lobby/v1/admin/notification/namespaces/{namespace}/topics` | Lists notification topics in the namespace |
| `notifications` | `list-topics` | public | v1 | GET | `/notification/namespaces/{namespace}/topics` | Lists notification topics in the namespace |
| `notifications` | `publish-template` | admin | v1 | POST | `/notification/namespaces/{namespace}/templates/{templateSlug}/languages/{templateLanguage}/publish` | Publishes a notification template draft |
| `notifications` | `publish-template-localization` | admin | v1 | POST | `/lobby/v1/admin/notification/namespaces/{namespace}/templates/{templateSlug}/languages/{templateLanguage}/publish` | Publishes a notification template localisation draft |
| `notifications` | `send` | admin | v1 | POST | `/lobby/v1/admin/notification/namespaces/{namespace}/users/{userId}/freeform/notify` | Sends a freeform notification to a specific user |
| `notifications` | `send` | public | v1 | POST | `/notification/namespaces/{namespace}/users/{userId}/freeform` | Sends a freeform notification to a user |
| `notifications` | `send-active` | admin | v1 | POST | `/lobby/v1/admin/notification/namespaces/{namespace}/freeform/notify` | Sends a freeform notification to all connected users |
| `notifications` | `send-active-templated` | admin | v1 | POST | `/lobby/v1/admin/notification/namespaces/{namespace}/templates/notify` | Sends a templated notification to all connected users |
| `notifications` | `send-connected` | admin | v1 | POST | `/notification/namespaces/{namespace}/freeform` | Sends a freeform notification to a user |
| `notifications` | `send-connected-templated` | admin | v1 | POST | `/notification/namespaces/{namespace}/templated` | Sends a templated notification to a user |
| `notifications` | `send-templated` | admin | v1 | POST | `/lobby/v1/admin/notification/namespaces/{namespace}/users/{userId}/templates/notify` | Sends a templated notification to a specific user |
| `notifications` | `send-templated` | public | v1 | POST | `/notification/namespaces/{namespace}/users/{userId}/templated` | Sends a templated notification to a user |
| `notifications` | `update-localized-template` | admin | v1 | PUT | `/notification/namespaces/{namespace}/templates/{templateSlug}/languages/{templateLanguage}` | Updates a notification template draft |
| `notifications` | `update-template-localization` | admin | v1 | PUT | `/lobby/v1/admin/notification/namespaces/{namespace}/templates/{templateSlug}/languages/{templateLanguage}` | Updates a notification template localisation |
| `notifications` | `update-topic` | admin | v1 | PUT | `/lobby/v1/admin/notification/namespaces/{namespace}/topics/{topicName}` | Updates information for a notification topic |
| `notifications` | `update-topic` | public | v1 | PUT | `/notification/namespaces/{namespace}/topics/{topic}` | Updates information for a notification topic |
| `presence` | `bulk-get` | public | v1 | POST | `/lobby/v1/public/presence/namespaces/{namespace}/users/presence` | Queries presence status for a list of users |
| `presence` | `list` | public | v1 | GET | `/lobby/v1/public/presence/namespaces/{namespace}/users/presence` | Queries presence status for a list of users |
| `service-messages` | `list` | public | v1 | GET | `/lobby/v1/messages` | Retrieves service messages |
| `users` | `block` | public | v1 | POST | `/lobby/v1/public/player/namespaces/{namespace}/users/me/block` | Blocks a player by user ID |
| `users` | `bulk-block` | admin | v1 | POST | `/lobby/v1/admin/player/namespaces/{namespace}/users/{userId}/bulk/block` | Blocks players in bulk for a specified user |
| `users` | `bulk-get-blocked` | admin | v1 | POST | `/lobby/v1/admin/player/namespaces/{namespace}/users/bulk/blocked` | Retrieves blocked players for a list of users |
| `users` | `bulk-unblock` | admin | v1 | DELETE | `/lobby/v1/admin/player/namespaces/{namespace}/users/{userId}/bulk/unblock` | Removes blocked players in bulk for a specified user |
| `users` | `get-ccu` | admin | v1 | GET | `/lobby/v1/admin/player/namespaces/{namespace}/ccu` | Retrieves the number of players currently connected to the lobby |
| `users` | `list-blocked` | admin | v1 | GET | `/lobby/v1/admin/player/namespaces/{namespace}/users/{userId}/blocked` | Retrieves blocked players for a specified user |
| `users` | `list-blocked` | public | v1 | GET | `/lobby/v1/public/player/namespaces/{namespace}/users/me/blocked` | Lists players blocked by the current user |
| `users` | `list-blocked-by` | admin | v1 | GET | `/lobby/v1/admin/player/namespaces/{namespace}/users/{userId}/blocked-by` | Retrieves players who have blocked a specified user |
| `users` | `list-blocked-by` | public | v1 | GET | `/lobby/v1/public/player/namespaces/{namespace}/users/me/blocked-by` | Lists players who have blocked the current user |
| `users` | `unblock` | public | v1 | POST | `/lobby/v1/public/player/namespaces/{namespace}/users/me/unblock` | Removes the block on a player by user ID |

## login-queue

- Spec name: `loginqueue`
- Resources: 1
- Operations: 5

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `queue` | `cancel-ticket` | public | v1 | DELETE | `/login-queue/v1/namespaces/{namespace}/ticket` | Cancels a login queue ticket |
| `queue` | `get-config` | admin | v1 | GET | `/login-queue/v1/admin/namespaces/{namespace}/config` | Retrieves login queue configuration |
| `queue` | `get-status` | admin | v1 | GET | `/login-queue/v1/admin/namespaces/{namespace}/status` | Retrieves login queue status |
| `queue` | `refresh-ticket` | public | v1 | GET | `/login-queue/v1/namespaces/{namespace}/ticket` | Refreshes a login queue ticket |
| `queue` | `update-config` | admin | v1 | PUT | `/login-queue/v1/admin/namespaces/{namespace}/config` | Updates login queue configuration |

## matchmaking

- Spec name: `match2`
- Resources: 8
- Operations: 42

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `backfill` | `accept` | public | v1 | PUT | `/match2/v1/namespaces/{namespace}/backfill/{backfillID}/proposal/accept` | Accepts a backfill proposal |
| `backfill` | `create` | public | v1 | POST | `/match2/v1/namespaces/{namespace}/backfill` | Creates a backfill ticket |
| `backfill` | `delete` | public | v1 | DELETE | `/match2/v1/namespaces/{namespace}/backfill/{backfillID}` | Deletes a backfill ticket |
| `backfill` | `get` | public | v1 | GET | `/match2/v1/namespaces/{namespace}/backfill/{backfillID}` | Retrieves a backfill ticket by ID |
| `backfill` | `get-proposal` | public | v1 | GET | `/match2/v1/namespaces/{namespace}/backfill/proposal` | Retrieves a backfill proposal for the session |
| `backfill` | `reject` | public | v1 | PUT | `/match2/v1/namespaces/{namespace}/backfill/{backfillID}/proposal/reject` | Rejects a backfill proposal |
| `backfill` | `search` | admin | v1 | GET | `/match2/v1/namespaces/{namespace}/backfill` | Queries backfill tickets in the namespace |
| `config` | `get` | admin | v1 | GET | `/match2/v1/config/namespaces/{namespace}` | Retrieves the matchmaking configuration for the namespace |
| `config` | `get-log` | admin | v1 | GET | `/match2/v1/admin/config/log` | Retrieves the log configuration |
| `config` | `get-version` | public | v1 | GET | `/match2/version` | Checks the service version |
| `config` | `list` | admin | v1 | GET | `/match2/v1/config` | Lists matchmaking configurations for all namespaces |
| `config` | `list-environment-variables` | admin | v1 | GET | `/match2/v1/environment-variables` | Lists environment variables |
| `config` | `update` | admin | v1 | PATCH | `/match2/v1/config/namespaces/{namespace}` | Updates the matchmaking configuration for the namespace |
| `config` | `update-log` | admin | v1 | PATCH | `/match2/v1/admin/config/log` | Updates the log configuration |
| `feature-flags` | `delete` | admin | v1 | DELETE | `/match2/v1/admin/namespaces/{namespace}/playfeatureflag` | Deletes the play feature flag for the namespace |
| `feature-flags` | `get` | admin | v1 | GET | `/match2/v1/admin/namespaces/{namespace}/playfeatureflag` | Retrieves the play feature flag for the namespace |
| `feature-flags` | `update` | admin | v1 | POST | `/match2/v1/admin/namespaces/{namespace}/playfeatureflag` | Creates or updates the play feature flag for the namespace |
| `match-functions` | `create` | admin | v1 | POST | `/match2/v1/namespaces/{namespace}/match-functions` | Creates a match function |
| `match-functions` | `delete` | admin | v1 | DELETE | `/match2/v1/namespaces/{namespace}/match-functions/{name}` | Deletes a match function |
| `match-functions` | `get` | admin | v1 | GET | `/match2/v1/namespaces/{namespace}/match-functions/{name}` | Retrieves a match function by name |
| `match-functions` | `list` | admin | v1 | GET | `/match2/v1/namespaces/{namespace}/match-functions` | Lists match functions in the namespace |
| `match-functions` | `update` | admin | v1 | PUT | `/match2/v1/namespaces/{namespace}/match-functions/{name}` | Updates a match function |
| `match-pools` | `create` | admin | v1 | POST | `/match2/v1/namespaces/{namespace}/match-pools` | Creates a match pool |
| `match-pools` | `create-error-metric` | admin | v1 | POST | `/match2/v1/namespaces/{namespace}/match-pools/{pool}/metrics/external-failure` | Records an external flow failure metric for a match pool |
| `match-pools` | `delete` | admin | v1 | DELETE | `/match2/v1/namespaces/{namespace}/match-pools/{pool}` | Deletes a match pool |
| `match-pools` | `get` | admin | v1 | GET | `/match2/v1/namespaces/{namespace}/match-pools/{pool}` | Retrieves details for a match pool |
| `match-pools` | `get-metric` | public | v1 | GET | `/match2/v1/namespaces/{namespace}/match-pools/{pool}/metrics` | Retrieves metrics for a match pool |
| `match-pools` | `get-user-metric` | admin | v1 | GET | `/match2/v1/namespaces/{namespace}/match-pools/{pool}/metrics/player` | Retrieves player metrics for a match pool |
| `match-pools` | `get-user-metric` | public | v1 | GET | `/match2/v1/public/namespaces/{namespace}/match-pools/{pool}/metrics/player` | Retrieves player metrics for a match pool |
| `match-pools` | `list` | admin | v1 | GET | `/match2/v1/namespaces/{namespace}/match-pools` | Lists match pools in the namespace |
| `match-pools` | `list-tickets` | admin | v1 | GET | `/match2/v1/namespaces/{namespace}/match-pools/{pool}/tickets` | Retrieves tickets in the queue for a match pool |
| `match-pools` | `update` | admin | v1 | PUT | `/match2/v1/namespaces/{namespace}/match-pools/{pool}` | Updates a match pool |
| `match-tickets` | `delete` | public | v1 | DELETE | `/match2/v1/namespaces/{namespace}/match-tickets/{ticketid}` | Deletes a matchmaking ticket |
| `match-tickets` | `get` | public | v1 | GET | `/match2/v1/namespaces/{namespace}/match-tickets/{ticketid}` | Retrieves details for a match ticket |
| `match-tickets` | `get-my` | public | v1 | GET | `/match2/v1/namespaces/{namespace}/match-tickets/me` | Retrieves match tickets for the current user |
| `rule-sets` | `create` | admin | v1 | POST | `/match2/v1/namespaces/{namespace}/rulesets` | Creates a match rule set |
| `rule-sets` | `delete` | admin | v1 | DELETE | `/match2/v1/namespaces/{namespace}/rulesets/{ruleset}` | Deletes a rule set |
| `rule-sets` | `get` | admin | v1 | GET | `/match2/v1/namespaces/{namespace}/rulesets/{ruleset}` | Retrieves details for a rule set |
| `rule-sets` | `list` | admin | v1 | GET | `/match2/v1/namespaces/{namespace}/rulesets` | Lists rule sets in the namespace |
| `rule-sets` | `update` | admin | v1 | PUT | `/match2/v1/namespaces/{namespace}/rulesets/{ruleset}` | Updates a match rule set |
| `xray-config` | `get` | admin | v1 | GET | `/match2/v1/admin/namespaces/{namespace}/xray/config` | Retrieves the X-Ray configuration for the namespace |
| `xray-config` | `update` | admin | v1 | POST | `/match2/v1/admin/namespaces/{namespace}/xray/config` | Updates the X-Ray configuration for the namespace |

## platform

- Spec name: `platform`
- Resources: 44
- Operations: 473

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `campaign-codes` | `bulk-disable` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/codes/campaigns/{campaignId}/disable/bulk` | Disables campaign codes in bulk |
| `campaign-codes` | `bulk-enable` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/codes/campaigns/{campaignId}/enable/bulk` | Enables campaign codes in bulk |
| `campaign-codes` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/codes/campaigns/{campaignId}` | Creates campaign codes |
| `campaign-codes` | `disable` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/codes/{code}/disable` | Disables a campaign code |
| `campaign-codes` | `enable` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/codes/{code}/enable` | Enables a campaign code |
| `campaign-codes` | `export` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/codes/campaigns/{campaignId}/codes.csv` | Downloads campaign codes as a CSV file |
| `campaign-codes` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/codes/{code}` | Retrieves campaign code information |
| `campaign-codes` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/codes/campaigns/{campaignId}` | Queries campaign codes |
| `campaign-codes` | `list-redemptions` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/codes/campaigns/{campaignId}/history` | Queries campaign code redemption history |
| `campaign-codes` | `redeem` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/redemption` | Redeems a campaign code for a user |
| `campaigns` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/campaigns` | Creates a campaign |
| `campaigns` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/campaigns/{campaignId}` | Retrieves a campaign by ID |
| `campaigns` | `get-statistics` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/campaigns/{campaignId}/dynamic` | Retrieves dynamic campaign statistics |
| `campaigns` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/campaigns` | Queries campaigns |
| `campaigns` | `list-batch-names` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/campaigns/{campaignId}/batchNames` | Queries campaign batch names |
| `campaigns` | `rename-batch` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/campaigns/{campaignId}/batchName` | Updates the batch name for a campaign |
| `campaigns` | `update` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/campaigns/{campaignId}` | Updates a campaign |
| `catalog-changes` | `get-statistics` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/stores/{storeId}/catalogChanges/statistics` | Retrieves catalog changes statistics |
| `catalog-changes` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/stores/{storeId}/catalogChanges/byCriteria` | Queries catalog changes |
| `catalog-changes` | `publish-all` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/stores/{storeId}/catalogChanges/publishAll` | Publishes all unpublished changes |
| `catalog-changes` | `publish-selected` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/stores/{storeId}/catalogChanges/publishSelected` | Publishes selected unpublished changes |
| `catalog-changes` | `select` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/stores/{storeId}/catalogChanges/{changeId}/select` | Selects a catalog change for partial publish |
| `catalog-changes` | `select-all` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/stores/{storeId}/catalogChanges/selectAll` | Selects all catalog changes |
| `catalog-changes` | `select-all-by-criteria` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/stores/{storeId}/catalogChanges/selectAllByCriteria` | Selects all catalog changes matching criteria |
| `catalog-changes` | `unselect` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/stores/{storeId}/catalogChanges/{changeId}/unselect` | Unselects a catalog change |
| `catalog-changes` | `unselect-all` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/stores/{storeId}/catalogChanges/unselectAll` | Unselects all catalog changes |
| `categories` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/categories` | Creates a category |
| `categories` | `delete` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/categories/{categoryPath}` | Deletes a category by path |
| `categories` | `download` | public | v1 | GET | `/platform/public/namespaces/{namespace}/categories/download` | Downloads the store's full category structure |
| `categories` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/categories/{categoryPath}` | Retrieves a category by path |
| `categories` | `get` | public | v1 | GET | `/platform/public/namespaces/{namespace}/categories/{categoryPath}` | Retrieves a category by path |
| `categories` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/categories` | Retrieves root categories |
| `categories` | `list` | public | v1 | GET | `/platform/public/namespaces/{namespace}/categories` | Retrieves root categories |
| `categories` | `list-basic` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/categories/basic` | Lists all categories with basic information |
| `categories` | `list-children` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/categories/{categoryPath}/children` | Retrieves child categories by parent path |
| `categories` | `list-children` | public | v1 | GET | `/platform/public/namespaces/{namespace}/categories/{categoryPath}/children` | Retrieves child categories for a given category path |
| `categories` | `list-descendants` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/categories/{categoryPath}/descendants` | Retrieves all descendant categories by path |
| `categories` | `list-descendants` | public | v1 | GET | `/platform/public/namespaces/{namespace}/categories/{categoryPath}/descendants` | Retrieves all descendant categories for a given category path |
| `categories` | `update` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/categories/{categoryPath}` | Updates a category |
| `clawback` | `get-history` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/clawback/histories` | Queries IAP clawback history |
| `clawback` | `mock-playstation-event` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/iap/clawback/playstation/mock` | Simulates a PlayStation clawback event for testing |
| `clawback` | `mock-xbox-event` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/iap/clawback/xbl/mock` | Simulates an Xbox clawback event for testing |
| `currencies` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/currencies` | Creates a currency |
| `currencies` | `delete` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/currencies/{currencyCode}` | Deletes a currency |
| `currencies` | `get-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/currencies/{currencyCode}/config` | Retrieves the configuration for a specific currency |
| `currencies` | `get-summary` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/currencies/{currencyCode}/summary` | Retrieves a summary of a specific currency |
| `currencies` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/currencies` | Lists currencies |
| `currencies` | `list` | public | v1 | GET | `/platform/public/namespaces/{namespace}/currencies` | Lists available currencies |
| `currencies` | `update` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/currencies/{currencyCode}` | Updates a currency |
| `dlc-config` | `delete` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/dlc/config/platformMap` | Deletes a platform DLC configuration |
| `dlc-config` | `delete-item` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/dlc/config/item` | Deletes a DLC item configuration |
| `dlc-config` | `get-durable-reward-map` | public | v1 | GET | `/platform/public/namespaces/{namespace}/dlc/rewards/durable/map` | Retrieves a DLC durable reward mapping |
| `dlc-config` | `get-item-history` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/dlc/config/history` | Retrieves the DLC item configuration history |
| `dlc-config` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/dlc/config/platformMap` | Retrieves the platform DLC configuration |
| `dlc-config` | `list-items` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/dlc/config/item` | Retrieves the DLC item configuration |
| `dlc-config` | `restore-item` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/dlc/config/history/{id}/restore` | Restores a DLC item configuration from a historical snapshot |
| `dlc-config` | `update` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/dlc/config/platformMap` | Updates the platform DLC configuration |
| `dlc-config` | `update-item` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/dlc/config/item` | Updates the DLC item configuration |
| `dlc-records` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/dlc/records` | Retrieves user DLC records |
| `dlc-records` | `list-by-platform` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/dlc` | Retrieves user DLC by platform |
| `dlc-records` | `list-content` | public | v1 | GET | `/platform/public/users/me/dlc/content` | Retrieves DLC reward contents for the current user |
| `dlc-records` | `sync-dlc` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/dlc/oculus/sync` | Syncs Oculus DLC entitlements for a user |
| `dlc-records` | `sync-epicgames` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/dlc/epicgames/sync` | Syncs Epic Games DLC entitlements for a user |
| `dlc-records` | `sync-playstation` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/dlc/psn/sync` | Syncs PSN Store DLC entitlements for a user |
| `dlc-records` | `sync-playstation-multi-label` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/dlc/psn/sync/multiServiceLabels` | Syncs PSN Store DLC entitlements across multiple service labels |
| `dlc-records` | `sync-steam` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/dlc/steam/sync` | Syncs Steam DLC entitlements for a user |
| `dlc-records` | `sync-xbox` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/dlc/xbl/sync` | Syncs Xbox DLC entitlements for a user |
| `entitlements` | `bulk-get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/byIds` | Retrieves user entitlements by IDs |
| `entitlements` | `bulk-get` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/byIds` | Retrieves user entitlements by their identifiers |
| `entitlements` | `bulk-grant` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/entitlements/grant` | Grants entitlements to multiple users in bulk |
| `entitlements` | `bulk-revoke` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/entitlements/revoke` | Revokes entitlements in bulk by their identifiers |
| `entitlements` | `check-my-ownership-any` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/me/entitlements/ownership/any` | Exists any my active entitlement |
| `entitlements` | `check-my-ownership-by-app-d` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/me/entitlements/ownership/byAppId` | Checks ownership of a specific app entitlement |
| `entitlements` | `check-my-ownership-by-item-id` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/me/entitlements/ownership/byItemId` | Checks ownership of a specific item entitlement |
| `entitlements` | `check-my-ownership-by-sku` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/me/entitlements/ownership/bySku` | Checks ownership of a specific SKU entitlement |
| `entitlements` | `check-ownership-any` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/ownership/any` | Exists any user active entitlement |
| `entitlements` | `check-ownership-any` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/ownership/any` | Exists any user active entitlement |
| `entitlements` | `check-ownership-any-item` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/ownership/anyOf` | Exists any user active entitlement |
| `entitlements` | `check-ownership-by-app-id` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/ownership/byAppId` | Retrieves user app entitlement ownership by app ID |
| `entitlements` | `check-ownership-by-app-id` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/ownership/byAppId` | Checks a user's ownership of a specific app entitlement |
| `entitlements` | `check-ownership-by-item-id` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/ownership/byItemId` | Retrieves user entitlement ownership by item ID |
| `entitlements` | `check-ownership-by-item-id` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/ownership/byItemId` | Checks a user's entitlement ownership for a specific item |
| `entitlements` | `check-ownership-by-item-ids` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/ownership/byItemIds` | Retrieves user entitlement ownership by item IDs |
| `entitlements` | `check-ownership-by-item-ids` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/ownership/byItemIds` | Checks a user's entitlement ownership for multiple items |
| `entitlements` | `check-ownership-by-sku` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/ownership/bySku` | Retrieves user entitlement ownership by SKU |
| `entitlements` | `check-ownership-by-sku` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/ownership/bySku` | Checks a user's entitlement ownership for a specific SKU |
| `entitlements` | `check-playstation-ownership` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/platforms/psn/entitlements/{entitlementLabel}/ownership` | Checks PSN entitlement ownership by entitlement label |
| `entitlements` | `check-revocation` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}/revoke/byUseCount/preCheck` | Checks if a user entitlement is eligible for use-count revocation |
| `entitlements` | `check-xbox-ownership` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/platforms/xbl/entitlements/{productSku}/ownership` | Checks Xbox entitlement ownership by product SKU |
| `entitlements` | `consume` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}/decrement` | Consumes a user entitlement |
| `entitlements` | `consume` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}/decrement` | Consumes a user entitlement |
| `entitlements` | `disable` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}/disable` | Disables a user entitlement |
| `entitlements` | `enable` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}/enable` | Enables a user entitlement |
| `entitlements` | `enable-origin` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/entitlements/config/entitlementOrigin/enable` | Enables the entitlement origin tracking feature |
| `entitlements` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}` | Retrieves a user entitlement |
| `entitlements` | `get` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}` | Retrieves a user entitlement by ID |
| `entitlements` | `get-app` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/byAppId` | Retrieves user app entitlement by app ID |
| `entitlements` | `get-app` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/byAppId` | Retrieves a user's app entitlement by app ID |
| `entitlements` | `get-by-id` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/entitlements/{entitlementId}` | Retrieves an entitlement by its identifier |
| `entitlements` | `get-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/platforms/{platform}/entitlement/config` | Retrieves the platform entitlement config list |
| `entitlements` | `get-config-info` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/entitlements/config/info` | Retrieves entitlement configuration information |
| `entitlements` | `get-history` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}/history` | Retrieves user entitlement history |
| `entitlements` | `get-history` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/history` | Retrieves entitlement history for a user |
| `entitlements` | `get-item` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/byItemId` | Retrieves user entitlement by item ID |
| `entitlements` | `get-my-ownership-token` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/me/entitlements/ownershipToken` | Gets an entitlement ownership token |
| `entitlements` | `get-sku` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/bySku` | Retrieves user entitlement by SKU |
| `entitlements` | `grant` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements` | Grants entitlements to a user |
| `entitlements` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements` | Queries user entitlements |
| `entitlements` | `list` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements` | Queries entitlements for a user |
| `entitlements` | `list-all` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/entitlements` | Queries entitlements across the namespace |
| `entitlements` | `list-all-by-item-ids` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/entitlements/byItemIds` | Queries entitlements filtered by item identifiers |
| `entitlements` | `list-by-app-type` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/byAppType` | Queries app entitlements by app type |
| `entitlements` | `list-by-app-type` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/byAppType` | Queries a user's app entitlements by application type |
| `entitlements` | `list-items` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/byItemIds` | Retrieves user entitlements by item IDs |
| `entitlements` | `revoke` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}/revoke` | Revokes a user entitlement |
| `entitlements` | `revoke-all` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/revoke` | Revokes all entitlements for a user |
| `entitlements` | `revoke-by-ids` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/revoke/byIds` | Revokes user entitlements by IDs |
| `entitlements` | `revoke-by-use-count` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}/revoke/byUseCount` | Revokes a specified use count of a user entitlement |
| `entitlements` | `sell` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}/sell` | Sells a user entitlement |
| `entitlements` | `sell` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}/sell` | Sells a user entitlement |
| `entitlements` | `split` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}/split` | Splits a user entitlement into two |
| `entitlements` | `transfer` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}/transfer` | Transfers use count from one entitlement to another |
| `entitlements` | `update` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/entitlements/{entitlementId}` | Updates a user entitlement |
| `entitlements` | `update-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/platforms/{platform}/entitlement/config` | Updates the platform entitlement config |
| `fulfillment-script` | `create` | admin | v1 | POST | `/platform/admin/fulfillment/scripts/{id}` | Creates a fulfillment script |
| `fulfillment-script` | `delete` | admin | v1 | DELETE | `/platform/admin/fulfillment/scripts/{id}` | Deletes a fulfillment script |
| `fulfillment-script` | `get` | admin | v1 | GET | `/platform/admin/fulfillment/scripts/{id}` | Retrieves a fulfillment script by ID |
| `fulfillment-script` | `list` | admin | v1 | GET | `/platform/admin/fulfillment/scripts` | Lists all fulfillment scripts |
| `fulfillment-script` | `update` | admin | v1 | PATCH | `/platform/admin/fulfillment/scripts/{id}` | Updates a fulfillment script |
| `fulfillments` | `check` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/fulfillment/preCheck` | Checks fulfillment items before processing |
| `fulfillments` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/fulfillment` | Fulfills an item for a user |
| `fulfillments` | `get-history` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/fulfillment/history` | Queries fulfillment histories in the namespace |
| `fulfillments` | `grant-rewards` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/fulfillment/rewards` | Fulfills rewards without content items |
| `fulfillments` | `grant-rewards` | admin | v2 | POST | `/platform/v2/admin/namespaces/{namespace}/users/{userId}/fulfillment/rewards` | Fulfills rewards for a user |
| `fulfillments` | `grant-transaction` | admin | v3 | PUT | `/platform/v3/admin/namespaces/{namespace}/users/{userId}/fulfillments/{transactionId}` | Fulfills items for a transaction |
| `fulfillments` | `list` | admin | v2 | GET | `/platform/v2/admin/namespaces/{namespace}/fulfillments` | Lists fulfillments in a namespace |
| `fulfillments` | `redeem-code` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/fulfillment/code` | Redeems a campaign code for a user |
| `fulfillments` | `redeem-code` | public | v1 | POST | `/platform/public/namespaces/{namespace}/users/{userId}/fulfillment/code` | Redeems a campaign code for a user |
| `fulfillments` | `retry` | admin | v3 | PUT | `/platform/v3/admin/namespaces/{namespace}/users/{userId}/fulfillments/{transactionId}/retry` | Retries a fulfillment by transaction ID |
| `fulfillments` | `revoke` | admin | v3 | PUT | `/platform/v3/admin/namespaces/{namespace}/users/{userId}/fulfillments/{transactionId}/revoke` | Revokes items for a transaction |
| `iap` | `delete-item` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/iap/config/item` | Deletes an IAP item configuration |
| `iap` | `get-consume-history` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/consume/history` | Queries IAP consumption history for a user |
| `iap` | `get-item-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/config/item` | Retrieves the IAP item configuration |
| `iap` | `get-item-mapping` | public | v1 | GET | `/platform/public/namespaces/{namespace}/iap/item/mapping` | Retrieves the IAP item mapping for a platform |
| `iap` | `get-order-details` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/{iapOrderNo}/consumedetails` | Retrieves IAP order consumption details by order number |
| `iap` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/iap` | Queries IAP orders for a user |
| `iap` | `list-all` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/all` | Queries all IAP orders for a user |
| `iap` | `list-order-line-items` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/orders/{iapOrderNo}/line_items` | Queries line items for an IAP order |
| `iap` | `mock-grant` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/mock/receipt` | Mocks IAP item fulfillment without receipt validation |
| `iap` | `refund` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/steam/orders/{iapOrderNo}/refund` | Refunds a Steam IAP order |
| `iap` | `update-item-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/item` | Updates the IAP item configuration |
| `iap-apple` | `delete-config` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/iap/config/apple` | Deletes the Apple IAP configuration |
| `iap-apple` | `get-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/config/apple` | Retrieves the Apple IAP configuration |
| `iap-apple` | `get-config-version` | public | v1 | GET | `/platform/public/namespaces/{namespace}/iap/apple/config/version` | Retrieves the Apple IAP configuration version |
| `iap-apple` | `grant` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/iap/apple/receipt` | Verifies an Apple IAP receipt and fulfills the item |
| `iap-apple` | `grant` | public | v2 | PUT | `/platform/v2/public/namespaces/{namespace}/users/{userId}/iap/apple/receipt` | Verifies an Apple IAP transaction and fulfills the item |
| `iap-apple` | `update-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/apple` | Updates the Apple IAP configuration |
| `iap-apple` | `upload-certificate` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/apple/cert` | Uploads an Apple Store p8 certificate file |
| `iap-epicgames` | `delete-config` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/iap/config/epicgames` | Deletes the Epic Games IAP configuration |
| `iap-epicgames` | `get-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/config/epicgames` | Retrieves the Epic Games IAP configuration |
| `iap-epicgames` | `sync-inventory` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/iap/epicgames/sync` | Syncs Epic Games inventory items for a user |
| `iap-epicgames` | `update-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/epicgames` | Updates the Epic Games IAP configuration |
| `iap-google` | `delete-config` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/iap/config/google` | Deletes the Google IAP configuration |
| `iap-google` | `get-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/config/google` | Retrieves the Google IAP configuration |
| `iap-google` | `grant` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/iap/google/receipt` | Verifies a Google IAP receipt and fulfills the item |
| `iap-google` | `update-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/google` | Updates the Google IAP configuration |
| `iap-google` | `upload-certificate` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/google/cert` | Uploads a Google Play p12 certificate file |
| `iap-notifications` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/notifications` | Queries third-party IAP notifications |
| `iap-oculus` | `delete-config` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/iap/config/oculus` | Deletes the Oculus IAP configuration |
| `iap-oculus` | `get-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/config/oculus` | Retrieves the Oculus IAP configuration |
| `iap-oculus` | `sync-consumable` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/iap/oculus/sync` | Syncs Oculus entitlements for a user |
| `iap-oculus` | `sync-subscriptions` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/oculus/subscription/sync` | Syncs Meta Quest subscription for a user |
| `iap-oculus` | `sync-subscriptions` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/iap/oculus/subscription/sync` | Syncs a Meta Quest subscription transaction for a user |
| `iap-oculus` | `update-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/oculus` | Updates the Oculus IAP configuration |
| `iap-playstation` | `delete-config` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/iap/config/playstation` | Deletes the PlayStation IAP configuration |
| `iap-playstation` | `get-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/config/playstation` | Retrieves the PlayStation IAP configuration |
| `iap-playstation` | `sync-store-entitlements` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/iap/psn/sync` | Syncs a user's PSN Store entitlements |
| `iap-playstation` | `sync-store-entitlements-multi` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/iap/psn/sync/multiServiceLabels` | Syncs PSN Store entitlements across multiple service labels |
| `iap-playstation` | `update-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/playstation` | Updates the PlayStation IAP configuration |
| `iap-playstation` | `validate-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/playstation/validate` | Validates a PlayStation IAP configuration |
| `iap-playstation` | `validate-existing-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/config/playstation/validate` | Validates the existing PlayStation IAP configuration |
| `iap-steam` | `delete-config` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/iap/config/steam` | Deletes the Steam IAP configuration |
| `iap-steam` | `get-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/config/steam` | Retrieves the Steam IAP configuration |
| `iap-steam` | `get-history` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/steam/report/histories` | Retrieves Steam IAP report processing histories |
| `iap-steam` | `get-job` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/steam/job` | Retrieves Steam report job information |
| `iap-steam` | `list-abnormal-transactions` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/steam/abnormal_transactions` | Queries Steam IAP transactions flagged as abnormal |
| `iap-steam` | `reset-job` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/steam/job/reset` | Resets the Steam report job to a specified time |
| `iap-steam` | `sync-abnormal-transactions` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/steam/syncAbnormalTransaction` | Syncs abnormal Steam transactions by transaction ID |
| `iap-steam` | `sync-abnormal-transactions` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/iap/steam/syncAbnormalTransaction` | Syncs abnormal Steam transactions when sync mode is TRANSACTION |
| `iap-steam` | `sync-inventory` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/iap/steam/sync` | Syncs Steam inventory items for a user |
| `iap-steam` | `sync-transaction` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/steam/syncByTransaction` | Manually syncs a Steam transaction by ID |
| `iap-steam` | `sync-transaction` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/iap/steam/syncByTransaction` | Syncs a Steam in-app purchase by transaction ID |
| `iap-steam` | `update-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/steam` | Updates the Steam IAP configuration |
| `iap-subscriptions` | `check-ownership-by-group-id` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/subscriptions/platforms/{platform}/ownership/byGroupId` | Retrieves user subscription ownership by subscription group ID |
| `iap-subscriptions` | `check-ownership-by-product-id` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/subscriptions/platforms/{platform}/ownership/byProductId` | Retrieves user subscription ownership by subscription product ID |
| `iap-subscriptions` | `create-oculus-group` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/iap/config/oculus/subscription/group` | Creates a Meta Quest subscription group |
| `iap-subscriptions` | `create-oculus-tier` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/iap/config/oculus/subscription/tier` | Creates a Meta Quest subscription tier |
| `iap-subscriptions` | `delete-oculus-group` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/iap/config/oculus/subscription/group/{sku}` | Deletes a Meta Quest subscription group |
| `iap-subscriptions` | `delete-oculus-tier` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/iap/config/oculus/subscription/tier/{sku}` | Deletes a Meta Quest subscription tier |
| `iap-subscriptions` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/subscriptions/{id}` | Retrieves user subscription details |
| `iap-subscriptions` | `get-transaction` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/subscriptions/transactions/{id}` | Retrieves details of a user subscription transaction |
| `iap-subscriptions` | `get-transaction-history` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/subscriptions/transactions/{id}/histories` | Retrieves subscription transaction update history |
| `iap-subscriptions` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/subscriptions` | Queries user subscriptions |
| `iap-subscriptions` | `list-all` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/subscriptions` | Queries IAP subscriptions |
| `iap-subscriptions` | `list-oculus-groups` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/config/oculus/subscription/group` | Lists all Meta Quest subscription groups |
| `iap-subscriptions` | `list-oculus-tiers` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/config/oculus/subscription/group/{sku}/tiers` | Lists all tiers for a Meta Quest subscription group |
| `iap-subscriptions` | `list-platform` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/iap/subscriptions/platforms/{platform}` | Retrieves a user's IAP subscriptions for a platform |
| `iap-subscriptions` | `list-stransactions` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/subscriptions/transactions` | Queries subscription transactions for a user |
| `iap-subscriptions` | `sync` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/subscriptions/{id}/sync` | Syncs subscription status for a user |
| `iap-subscriptions` | `sync-transaction` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/iap/subscriptions/transactions/{id}/sync` | Syncs a subscription transaction |
| `iap-twitch` | `delete-config` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/iap/config/twitch` | Deletes the Twitch IAP configuration |
| `iap-twitch` | `get-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/config/twitch` | Retrieves the Twitch IAP configuration |
| `iap-twitch` | `sync-drops` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/iap/twitch/sync` | Syncs Twitch Drops entitlements for a user |
| `iap-twitch` | `sync-my-drops` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/me/iap/twitch/sync` | Syncs Twitch drop entitlements for the current user |
| `iap-twitch` | `update-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/twitch` | Updates the Twitch IAP configuration |
| `iap-xbox` | `delete-config` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/iap/config/xbl` | Deletes the Xbox Live IAP configuration |
| `iap-xbox` | `get-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/iap/config/xbl` | Retrieves the Xbox Live IAP configuration |
| `iap-xbox` | `sync-inventory` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/iap/xbl/sync` | Syncs Xbox inventory items for a user |
| `iap-xbox` | `update-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/xbl` | Updates the Xbox Live IAP configuration |
| `iap-xbox` | `upload-certificate` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/iap/config/xbl/cert` | Uploads an Xbox Live business partner certificate file |
| `invoices` | `download` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/invoice/details.csv` | Downloads invoice details as a CSV file |
| `invoices` | `generate-summary` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/invoice/summary` | Generates an invoice summary |
| `items` | `acquire` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/items/{itemId}/acquire` | Acquires a published item and decrements its available sale count |
| `items` | `add-feature` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/items/{itemId}/features/{feature}` | Adds a feature tag to an item |
| `items` | `bulk-update-region-data` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/items/regiondata` | Updates region data for items in bulk |
| `items` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/items` | Creates an item in the store |
| `items` | `create-type-config` | admin | v1 | POST | `/platform/admin/items/configs` | Creates an item type configuration |
| `items` | `delete` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/items/{itemId}` | Deletes an item permanently |
| `items` | `delete-type-config` | admin | v1 | DELETE | `/platform/admin/items/configs/{id}` | Deletes an item type configuration permanently |
| `items` | `disable` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/items/{itemId}/disable` | Disables an item in the store |
| `items` | `enable` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/items/{itemId}/enable` | Enables an item in the store |
| `items` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/{itemId}` | Retrieves an item by identifier |
| `items` | `get` | public | v1 | GET | `/platform/public/namespaces/{namespace}/items/{itemId}/locale` | Retrieves an item with locale-specific details |
| `items` | `get-app` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/{itemId}/app` | Retrieves app info for an item |
| `items` | `get-app` | public | v1 | GET | `/platform/public/namespaces/{namespace}/items/{itemId}/app/locale` | Retrieves an app item with locale-specific details |
| `items` | `get-by-app-id` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/byAppId` | Retrieves an item by its application identifier |
| `items` | `get-by-app-id` | public | v1 | GET | `/platform/public/namespaces/{namespace}/items/byAppId` | Retrieves an item by its app ID |
| `items` | `get-by-sku` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/bySku` | Retrieves an item by its SKU |
| `items` | `get-by-sku` | public | v1 | GET | `/platform/public/namespaces/{namespace}/items/bySku` | Retrieves an item by its SKU |
| `items` | `get-dynamic-data` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/{itemId}/dynamic` | Retrieves dynamic data for a published item |
| `items` | `get-dynamic-data` | public | v1 | GET | `/platform/public/namespaces/{namespace}/items/{itemId}/dynamic` | Retrieves dynamic data for an item |
| `items` | `get-estimated-price` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/estimatedPrice` | Retrieves estimated prices for flexible pricing bundle items |
| `items` | `get-estimated-price` | public | v1 | GET | `/platform/public/namespaces/{namespace}/items/estimatedPrice` | Retrieves estimated prices for items |
| `items` | `get-id-by-sku` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/itemId/bySku` | Retrieves an item identifier by SKU |
| `items` | `get-locale` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/{itemId}/locale` | Retrieves an item in the specified locale |
| `items` | `get-locale-by-sku` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/bySku/locale` | Retrieves an item by SKU with locale-specific content |
| `items` | `get-type-config` | admin | v1 | GET | `/platform/admin/items/configs/{id}` | Retrieves an item type configuration by ID |
| `items` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/byCriteria` | Queries items by criteria within a store |
| `items` | `list` | admin | v2 | GET | `/platform/v2/admin/namespaces/{namespace}/items/byCriteria` | Queries items by criteria within a store |
| `items` | `list` | public | v1 | GET | `/platform/public/namespaces/{namespace}/items/byCriteria` | Queries store items by criteria |
| `items` | `list-basic-by-features` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/byFeatures/basic` | Lists basic items filtered by features |
| `items` | `list-by-user-id` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/byIds` | Retrieves items by their identifiers |
| `items` | `list-id-by-skus` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/itemId/bySkus` | Retrieves item IDs for a list of SKUs within a store |
| `items` | `list-locale` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/locale/byIds` | Retrieves multiple items with localized content for a specific language and region |
| `items` | `list-locale` | public | v1 | GET | `/platform/public/namespaces/{namespace}/items/locale/byIds` | Retrieves locale-specific item details in bulk |
| `items` | `list-predicate-types` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/predicate/types` | Retrieves available predicate types for items |
| `items` | `list-references` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/{itemId}/references` | Retrieves references for an item |
| `items` | `list-type-config` | admin | v1 | GET | `/platform/admin/items/configs` | Lists all item type configurations |
| `items` | `list-uncategorized` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/uncategorized` | Queries uncategorized items in a store |
| `items` | `remove-feature` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/items/{itemId}/features/{feature}` | Removes a feature tag from an item |
| `items` | `return` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/items/{itemId}/return` | Returns a purchased item and increments its available sale count |
| `items` | `search` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/items/search` | Searches items by keyword |
| `items` | `search` | public | v1 | GET | `/platform/public/namespaces/{namespace}/items/search` | Searches store items by keyword |
| `items` | `search-type-config` | admin | v1 | GET | `/platform/admin/items/configs/search` | Searches for an item type configuration by item type and class |
| `items` | `sync-in-game` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/items` | Syncs an in-game item |
| `items` | `update` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/items/{itemId}` | Updates an item in the store |
| `items` | `update-app` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/items/{itemId}/app` | Updates app properties for an item |
| `items` | `update-purchase-condition` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/items/{itemId}/purchase/condition` | Updates the purchase condition for an item |
| `items` | `update-type-config` | admin | v1 | PUT | `/platform/admin/items/configs/{id}` | Updates an item type configuration |
| `items` | `validate-purchase-condition` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/items/purchase/conditions/validate` | Validates purchase conditions for a user |
| `items` | `validate-purchase-condition` | public | v1 | POST | `/platform/public/namespaces/{namespace}/items/purchase/conditions/validate` | Validates whether a user meets the purchase conditions for items |
| `key-groups` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/keygroups` | Creates a key group |
| `key-groups` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/keygroups/{keyGroupId}` | Retrieves a key group by identifier |
| `key-groups` | `get-dynamic` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/keygroups/{keyGroupId}/dynamic` | Retrieves dynamic data for a key group |
| `key-groups` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/keygroups` | Queries key groups by name or tag |
| `key-groups` | `list-keys` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/keygroups/{keyGroupId}/keys` | Lists keys in a key group |
| `key-groups` | `update` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/keygroups/{keyGroupId}` | Updates a key group |
| `key-groups` | `upload-keys` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/keygroups/{keyGroupId}/keys` | Uploads keys from a CSV file to a key group |
| `orders` | `cancel` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/orders/{orderNo}/cancel` | Cancels a user order |
| `orders` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/orders` | Creates an order for a user |
| `orders` | `create` | public | v1 | POST | `/platform/public/namespaces/{namespace}/users/{userId}/orders` | Creates an order for a user |
| `orders` | `download-receipt` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/orders/{orderNo}/receipt.pdf` | Downloads an order receipt as PDF |
| `orders` | `download-receipt` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/orders/{orderNo}/receipt.pdf` | Downloads the receipt PDF for a user order |
| `orders` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/orders/{orderNo}` | Retrieves an order by order number |
| `orders` | `get-history` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/orders/{orderNo}/history` | Retrieves order history for a user |
| `orders` | `get-history` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/orders/{orderNo}/history` | Retrieves the history of a user order |
| `orders` | `get-item-count` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/orders/countOfItem` | Retrieves the count of purchased items for a user |
| `orders` | `get-statistics` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/orders/stats` | Retrieves order statistics for a namespace |
| `orders` | `get-user` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/orders/{orderNo}` | Retrieves an order |
| `orders` | `get-user` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/orders/{orderNo}` | Retrieves a user order by order number |
| `orders` | `grant` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/orders/{orderNo}/fulfill` | Fulfills a charged but unfulfilled order |
| `orders` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/orders` | Queries orders in a namespace |
| `orders` | `list-user` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/orders` | Queries orders for a user |
| `orders` | `list-user` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/orders` | Lists a user's orders |
| `orders` | `preview-price` | public | v1 | POST | `/platform/public/namespaces/{namespace}/users/{userId}/orders/discount/preview` | Previews an order price after applying a discount code |
| `orders` | `process-notification` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/orders/{orderNo}/notifications` | Processes a payment notification webhook |
| `orders` | `refund` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/orders/{orderNo}/refund` | Refunds an order |
| `orders` | `sync` | admin | v1 | GET | `/platform/admin/orders` | Syncs orders within a time range |
| `orders` | `update-status` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/orders/{orderNo}` | Updates the status of an order |
| `payment` | `charge` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/payment/orders/{paymentOrderNo}` | Charges a payment order directly without the payment flow |
| `payment` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/payment/orders` | Creates a payment order for a user |
| `payment` | `get-order` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/payment/orders/{paymentOrderNo}` | Retrieves a payment order by order number |
| `payment` | `get-status` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/payment/orders/{paymentOrderNo}/status` | Retrieves the charge status of a payment order |
| `payment` | `list-notifications` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/payment/notifications` | Queries payment notifications |
| `payment` | `list-orders` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/payment/orders` | Queries payment orders |
| `payment` | `list-orders-by-external-transaction-id` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/payment/orders/byExtTxId` | Lists payment order numbers by external transaction ID |
| `payment` | `mock-notification` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/payment/orders/{paymentOrderNo}/simulate-notification` | Simulates a payment notification for a sandbox order |
| `payment` | `refund-order` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/payment/orders/{paymentOrderNo}/refund` | Refunds a payment order |
| `payment-accounts` | `delete` | public | v1 | DELETE | `/platform/public/namespaces/{namespace}/users/{userId}/payment/accounts/{type}/{id}` | Deletes a saved payment account |
| `payment-accounts` | `list` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/payment/accounts` | Retrieves a user's saved payment accounts |
| `payment-config` | `check-adyen` | admin | v1 | POST | `/platform/admin/payment/config/merchant/adyenconfig/test` | Tests the Adyen payment provider configuration |
| `payment-config` | `check-adyen-by-id` | admin | v1 | GET | `/platform/admin/payment/config/merchant/{id}/adyenconfig/test` | Tests the Adyen configuration for a payment merchant |
| `payment-config` | `check-alipay` | admin | v1 | POST | `/platform/admin/payment/config/merchant/alipayconfig/test` | Tests the Alipay payment provider configuration |
| `payment-config` | `check-alipay-by-id` | admin | v1 | GET | `/platform/admin/payment/config/merchant/{id}/alipayconfig/test` | Tests the Alipay configuration for a payment merchant |
| `payment-config` | `check-checkout` | admin | v1 | POST | `/platform/admin/payment/config/merchant/checkoutconfig/test` | Tests the Checkout.com payment provider configuration |
| `payment-config` | `check-checkout-by-id` | admin | v1 | GET | `/platform/admin/payment/config/merchant/{id}/checkoutconfig/test` | Tests the Checkout.com configuration for a payment merchant |
| `payment-config` | `check-neonpay` | admin | v1 | POST | `/platform/admin/payment/config/merchant/neonpayconfig/test` | Tests the Neon Pay payment provider configuration |
| `payment-config` | `check-neonpay-by-id` | admin | v1 | GET | `/platform/admin/payment/config/merchant/{id}/neonpayconfig/test` | Tests the Neon Pay configuration for a payment merchant |
| `payment-config` | `check-paypal` | admin | v1 | POST | `/platform/admin/payment/config/merchant/paypalconfig/test` | Tests the PayPal payment provider configuration |
| `payment-config` | `check-paypal-by-id` | admin | v1 | GET | `/platform/admin/payment/config/merchant/{id}/paypalconfig/test` | Tests the PayPal configuration for a payment merchant |
| `payment-config` | `check-stripe` | admin | v1 | POST | `/platform/admin/payment/config/merchant/stripeconfig/test` | Tests the Stripe payment provider configuration |
| `payment-config` | `check-stripe-by-id` | admin | v1 | GET | `/platform/admin/payment/config/merchant/{id}/stripeconfig/test` | Tests the Stripe configuration for a payment merchant |
| `payment-config` | `check-wxpay` | admin | v1 | POST | `/platform/admin/payment/config/merchant/wxpayconfig/test` | Tests the WxPay payment provider configuration |
| `payment-config` | `check-wxpay-by-id` | admin | v1 | GET | `/platform/admin/payment/config/merchant/{id}/wxpayconfig/test` | Tests the WxPay configuration for a payment merchant |
| `payment-config` | `check-xsolla` | admin | v1 | POST | `/platform/admin/payment/config/merchant/xsollaconfig/test` | Tests the Xsolla payment provider configuration |
| `payment-config` | `check-xsolla-by-id` | admin | v1 | GET | `/platform/admin/payment/config/merchant/{id}/xsollaconfig/test` | Tests the Xsolla configuration for a payment merchant |
| `payment-config` | `create-provider` | admin | v1 | POST | `/platform/admin/payment/config/provider` | Creates a payment provider configuration |
| `payment-config` | `delete-provider` | admin | v1 | DELETE | `/platform/admin/payment/config/provider/{id}` | Deletes a payment provider configuration |
| `payment-config` | `get-domain-allow-list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/payment/config/domains` | Retrieves the payment domain whitelist config for a namespace |
| `payment-config` | `get-matched-merchant` | admin | v1 | GET | `/platform/admin/payment/config/merchant/matched` | Retrieves the matched payment merchant configuration for a region |
| `payment-config` | `get-matched-provider` | admin | v1 | GET | `/platform/admin/payment/config/provider/matched` | Retrieves the matched payment provider configuration for a namespace and region |
| `payment-config` | `get-merchant` | admin | v1 | GET | `/platform/admin/payment/config/merchant/{id}` | Retrieves a payment merchant configuration |
| `payment-config` | `get-tax` | admin | v1 | GET | `/platform/admin/payment/config/tax` | Retrieves the global payment tax configuration |
| `payment-config` | `list-aggregate-providers` | admin | v1 | GET | `/platform/admin/payment/config/provider/aggregate` | Retrieves available aggregate payment providers |
| `payment-config` | `list-providers` | admin | v1 | GET | `/platform/admin/payment/config/provider` | Queries payment provider configurations |
| `payment-config` | `list-special-providers` | admin | v1 | GET | `/platform/admin/payment/config/provider/special` | Retrieves available special payment providers |
| `payment-config` | `update-adyen` | admin | v1 | PUT | `/platform/admin/payment/config/merchant/{id}/adyenconfig` | Updates the Adyen configuration for a payment merchant |
| `payment-config` | `update-alipay` | admin | v1 | PUT | `/platform/admin/payment/config/merchant/{id}/alipayconfig` | Updates the Alipay configuration for a payment merchant |
| `payment-config` | `update-checkout` | admin | v1 | PUT | `/platform/admin/payment/config/merchant/{id}/checkoutconfig` | Updates the Checkout.com configuration for a payment merchant |
| `payment-config` | `update-domain-allow-list` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/payment/config/domains` | Updates the payment domain whitelist config for a namespace |
| `payment-config` | `update-neonpay` | admin | v1 | PUT | `/platform/admin/payment/config/merchant/{id}/neonpayconfig` | Updates the Neon Pay configuration for a payment merchant |
| `payment-config` | `update-paypal` | admin | v1 | PUT | `/platform/admin/payment/config/merchant/{id}/paypalconfig` | Updates the PayPal configuration for a payment merchant |
| `payment-config` | `update-provider` | admin | v1 | PUT | `/platform/admin/payment/config/provider/{id}` | Updates a payment provider configuration |
| `payment-config` | `update-stripe` | admin | v1 | PUT | `/platform/admin/payment/config/merchant/{id}/stripeconfig` | Updates the Stripe configuration for a payment merchant |
| `payment-config` | `update-tax` | admin | v1 | PUT | `/platform/admin/payment/config/tax` | Updates the global payment tax configuration |
| `payment-config` | `update-wxpay` | admin | v1 | PUT | `/platform/admin/payment/config/merchant/{id}/wxpayconfig` | Updates the WxPay configuration for a payment merchant |
| `payment-config` | `update-wxpay-certificate` | admin | v1 | PUT | `/platform/admin/payment/config/merchant/{id}/wxpayconfig/cert` | Uploads the WxPay certificate file for a payment merchant |
| `payment-config` | `update-xsolla` | admin | v1 | PUT | `/platform/admin/payment/config/merchant/{id}/xsollaconfig` | Updates the Xsolla configuration for a payment merchant |
| `payment-config` | `update-xsolla-ui` | admin | v1 | PUT | `/platform/admin/payment/config/merchant/{id}/xsollauiconfig` | Updates the Xsolla UI configuration for a payment merchant |
| `payment-station` | `check-paid` | public | v1 | GET | `/platform/public/namespaces/{namespace}/payment/orders/{paymentOrderNo}/status` | Checks the paid status of a payment order |
| `payment-station` | `get-methods` | public | v1 | GET | `/platform/public/namespaces/{namespace}/payment/methods` | Retrieves available payment methods for a payment order |
| `payment-station` | `get-public-config` | public | v1 | GET | `/platform/public/namespaces/{namespace}/payment/publicconfig` | Retrieves the public configuration for a payment provider |
| `payment-station` | `get-qr-code` | public | v1 | GET | `/platform/public/namespaces/{namespace}/payment/qrcode` | Retrieves a QR code for a payment |
| `payment-station` | `get-tax` | public | v1 | GET | `/platform/public/namespaces/{namespace}/payment/tax` | Retrieves the tax calculation result for a payment order |
| `payment-station` | `get-unpaid` | public | v1 | GET | `/platform/public/namespaces/{namespace}/payment/orders/{paymentOrderNo}/info` | Retrieves payment order information |
| `payment-station` | `get-url` | public | v1 | POST | `/platform/public/namespaces/{namespace}/payment/link` | Retrieves a payment URL for a payment order |
| `payment-station` | `normalized-return-url` | public | v1 | GET | `/platform/public/namespaces/{namespace}/payment/returnurl` | Normalizes a payment provider return URL after payment |
| `payment-station` | `pay` | public | v1 | POST | `/platform/public/namespaces/{namespace}/payment/orders/{paymentOrderNo}/pay` | Submits payment for a payment order |
| `payments` | `create-platform-order` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/payment/orders` | Creates a payment order from a non-Justice service |
| `payments` | `refund-platform-order` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/payment/orders/{paymentOrderNo}/refund` | Refunds a payment order from a non-Justice service |
| `payments` | `sync-platform-orders` | admin | v1 | GET | `/platform/admin/payment/orders` | Syncs payment orders within a time range |
| `platform-achievements` | `get-steam` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/achievement/steam` | Unlocks a Steam achievement for a user |
| `platform-achievements` | `get-xbox` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/achievement/xbl` | Retrieves Xbox Live achievements for a user |
| `platform-achievements` | `update-xbox` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/achievement/xbl` | Updates Xbox Live achievements for a user |
| `platform-closure` | `get-user-history` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/platform/closure/history` | Retrieves platform account closure history for a user |
| `platform-sessions` | `register-box` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/session/xbl` | Registers or updates an Xbox session for a user |
| `plugin-config` | `delete-loot-box` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/catalog/plugins/lootbox` | Deletes the lootbox plugin configuration |
| `plugin-config` | `delete-loot-section` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/catalog/plugins/section` | Deletes the section plugin configuration |
| `plugin-config` | `delete-revocation` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/revocation/plugins/revocation` | Deletes the revocation plugin config |
| `plugin-config` | `get-loot-box` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/catalog/plugins/lootbox` | Retrieves the lootbox plugin configuration |
| `plugin-config` | `get-loot-box-grpc` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/catalog/plugins/lootbox/grpcInfo` | Retrieves lootbox plugin gRPC server information |
| `plugin-config` | `get-revocation` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/revocation/plugins/revocation` | Retrieves the revocation plugin config |
| `plugin-config` | `get-section` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/catalog/plugins/section` | Retrieves the section plugin configuration |
| `plugin-config` | `update-loot-box` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/catalog/plugins/lootbox` | Updates the lootbox plugin configuration |
| `plugin-config` | `update-revocation` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/revocation/plugins/revocation` | Updates the revocation plugin config |
| `plugin-config` | `update-section` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/catalog/plugins/section` | Updates the section plugin configuration |
| `plugin-config` | `upload-loot-box-certificate` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/catalog/plugins/lootbox/customConfig/cert` | Uploads the lootbox plugin TLS certificate |
| `plugin-config` | `upload-revocation-certificate` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/revocation/plugins/revocation/revocation/customConfig/cert` | Uploads the TLS certificate for the revocation plugin custom config |
| `plugin-config` | `upload-section-certificate` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/catalog/plugins/section/customConfig/cert` | Uploads the section plugin TLS certificate |
| `revocations` | `delete-config` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/revocation/config` | Deletes the revocation config for a namespace |
| `revocations` | `execute` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/revocation` | Performs a revocation for a user |
| `revocations` | `get-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/revocation/config` | Retrieves the revocation configuration for a namespace |
| `revocations` | `get-history` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/revocation/history` | Queries revocation histories in a namespace |
| `revocations` | `update-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/revocation/config` | Updates the revocation configuration for a namespace |
| `rewards` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/rewards` | Creates a reward |
| `rewards` | `delete` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/rewards/{rewardId}` | Deletes a reward permanently |
| `rewards` | `delete-condition` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/rewards/{rewardId}/record` | Deletes a reward condition record |
| `rewards` | `export` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/rewards/export` | Exports all reward configurations for a namespace |
| `rewards` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/rewards/{rewardId}` | Retrieves a reward by identifier |
| `rewards` | `get` | public | v1 | GET | `/platform/public/namespaces/{namespace}/rewards/{rewardId}` | Retrieves a reward by its identifier |
| `rewards` | `get-by-code` | public | v1 | GET | `/platform/public/namespaces/{namespace}/rewards/byCode` | Retrieves a reward by its code |
| `rewards` | `import` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/rewards/import` | Imports reward configurations for a namespace |
| `rewards` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/rewards/byCriteria` | Queries rewards by criteria |
| `rewards` | `list` | public | v1 | GET | `/platform/public/namespaces/{namespace}/rewards/byCriteria` | Queries rewards by criteria |
| `rewards` | `match-event` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/rewards/{rewardId}/match` | Checks if an event matches a reward condition |
| `rewards` | `update` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/rewards/{rewardId}` | Updates a reward |
| `sections` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/sections` | Creates a section |
| `sections` | `delete` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/sections/{sectionId}` | Deletes a section permanently |
| `sections` | `delete-expired` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/sections/purge/expired` | Purges expired sections from a store |
| `sections` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/sections/{sectionId}` | Retrieves a section by identifier |
| `sections` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/sections` | Lists sections in a namespace |
| `sections` | `list-active` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/sections` | Lists active section contents |
| `sections` | `update` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/sections/{sectionId}` | Updates a section |
| `stores` | `clone` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/stores/{storeId}/clone` | Clones a store |
| `stores` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/stores` | Creates a store |
| `stores` | `delete` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/stores/{storeId}` | Deletes a store |
| `stores` | `delete-publisheed` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/stores/published` | Deletes the published store |
| `stores` | `download-template` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/stores/downloadCSVTemplates` | Downloads CSV templates for store import |
| `stores` | `export` | admin | v2 | POST | `/platform/v2/admin/namespaces/{namespace}/stores/{storeId}/export` | Exports a store's items |
| `stores` | `export-csv` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/stores/exportByCSV` | Exports a store to CSV format |
| `stores` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/stores/{storeId}` | Retrieves a store |
| `stores` | `get-backup` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/stores/published/backup` | Retrieves the published store's backup |
| `stores` | `get-catalog-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/catalog/configs` | Retrieves the catalog configuration |
| `stores` | `get-catalog-definition` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/stores/catalogDefinition` | Retrieves the catalog definition |
| `stores` | `get-import-history` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/stores/{storeId}/import/history` | Queries store import history |
| `stores` | `get-published` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/stores/published` | Retrieves the published store |
| `stores` | `import` | admin | v2 | PUT | `/platform/v2/admin/namespaces/{namespace}/stores/import` | Imports a store from a file |
| `stores` | `import-csv` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/stores/{storeId}/importByCSV` | Imports a store from CSV format |
| `stores` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/stores` | Lists stores in a namespace |
| `stores` | `list` | public | v1 | GET | `/platform/public/namespaces/{namespace}/stores` | Lists all stores in a namespace |
| `stores` | `rollback` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/stores/published/rollback` | Rolls back the published store |
| `stores` | `update` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/stores/{storeId}` | Updates a store |
| `stores` | `update-catalog-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/catalog/configs` | Updates the catalog configuration |
| `subscriptions` | `cancel` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/subscriptions/{subscriptionId}/cancel` | Cancels a user subscription |
| `subscriptions` | `cancel` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/subscriptions/{subscriptionId}/cancel` | Cancels a subscription |
| `subscriptions` | `charge-recurring` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/subscriptions/{subscriptionId}/recurring` | Triggers a recurring charge for a subscription |
| `subscriptions` | `check-subscribable-item` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/subscriptions/subscribable/byItemId` | Checks if a user can subscribe to an item |
| `subscriptions` | `check-subscribable-item` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/subscriptions/subscribable/byItemId` | Checks whether a user can subscribe to an item |
| `subscriptions` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/subscriptions/platformSubscribe` | Subscribes a user for free via a platform |
| `subscriptions` | `create` | public | v1 | POST | `/platform/public/namespaces/{namespace}/users/{userId}/subscriptions` | Creates a subscription for a user |
| `subscriptions` | `delete` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/users/{userId}/subscriptions/{subscriptionId}` | Deletes a user subscription |
| `subscriptions` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/subscriptions/{subscriptionId}` | Retrieves a user subscription |
| `subscriptions` | `get` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/subscriptions/{subscriptionId}` | Retrieves a user subscription by ID |
| `subscriptions` | `get-billing-history` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/subscriptions/{subscriptionId}/history` | Retrieves subscription billing history for a user |
| `subscriptions` | `get-billing-history` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/subscriptions/{subscriptionId}/history` | Retrieves billing history for a subscription |
| `subscriptions` | `grant-days` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/subscriptions/{subscriptionId}/grant` | Grants additional days to a subscription |
| `subscriptions` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/subscriptions` | Queries subscriptions for a user |
| `subscriptions` | `list` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/subscriptions` | Lists a user's subscriptions |
| `subscriptions` | `list-activities` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/subscriptions/activities` | Retrieves subscription activity for a user |
| `subscriptions` | `list-all` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/subscriptions` | Queries subscriptions |
| `subscriptions` | `process-notification` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/subscriptions/{subscriptionId}/notifications` | Processes a subscription payment notification webhook |
| `subscriptions` | `update-billing-account` | public | v1 | PUT | `/platform/public/namespaces/{namespace}/users/{userId}/subscriptions/{subscriptionId}/billingAccount` | Requests a billing account change for a subscription |
| `tickets` | `acquire` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/tickets/{boothName}` | Acquires a ticket from a booth for a user |
| `tickets` | `decrement` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/tickets/{boothName}/decrement` | Decrements ticket sale count |
| `tickets` | `get-booth-id` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/tickets/{boothName}/id` | Retrieves the ticket booth ID |
| `tickets` | `get-dynamic` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/tickets/{boothName}` | Retrieves ticket dynamic data |
| `tickets` | `increment` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/tickets/{boothName}/increment` | Increments ticket sale count |
| `trades` | `commit` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/trade/commit` | Commits a chain of trade operations |
| `trades` | `get-history-by-criteria` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/trade/history/byCriteria` | Retrieves trade history by criteria |
| `trades` | `get-history-by-transaction-id` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/trade/{transactionId}` | Retrieves trade history by transaction ID |
| `views` | `create` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/views` | Creates a view |
| `views` | `delete` | admin | v1 | DELETE | `/platform/admin/namespaces/{namespace}/views/{viewId}` | Deletes a view |
| `views` | `get` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/views/{viewId}` | Retrieves a view |
| `views` | `list` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/views` | Lists all views in a store |
| `views` | `list` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/views` | Lists all views for a user |
| `views` | `update` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/views/{viewId}` | Updates a view |
| `wallets` | `bulk-credit` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/wallets/credit` | Credits multiple users' wallets in bulk |
| `wallets` | `bulk-debit` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/wallets/debit` | Debits multiple users' wallets in bulk |
| `wallets` | `credit` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/wallets/{currencyCode}/credit` | Credits a user wallet by currency code and balance origin |
| `wallets` | `debit-currency` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/wallets/currencies/{currencyCode}/debit` | Debits a user wallet by currency code |
| `wallets` | `debit-platform` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/wallets/{currencyCode}/debitByWalletPlatform` | Debits a user wallet by currency code and client platform |
| `wallets` | `get` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/wallets/{currencyCode}` | Retrieves a wallet by currency code |
| `wallets` | `get-balance` | admin | v1 | POST | `/platform/admin/namespaces/{namespace}/users/{userId}/wallets/{currencyCode}/balanceCheck` | Checks if a user has sufficient wallet balance |
| `wallets` | `get-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/wallet/config` | Retrieves the wallet configuration for a namespace |
| `wallets` | `get-currency-summary` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/wallets/currencies/summary` | Retrieves currency wallet summary for a user |
| `wallets` | `get-my-by-currency` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/me/wallets/{currencyCode}` | Retrieves the current user's wallet for a currency |
| `wallets` | `get-platform-config` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/platforms/{platform}/wallet/config` | Retrieves the platform wallet config list |
| `wallets` | `list-transactions` | public | v1 | GET | `/platform/public/namespaces/{namespace}/users/{userId}/wallets/{currencyCode}/transactions` | Lists wallet transactions for a currency code |
| `wallets` | `list-transactions-by-currency` | admin | v1 | GET | `/platform/admin/namespaces/{namespace}/users/{userId}/wallets/currencies/{currencyCode}/transactions` | Lists currency transactions for a user |
| `wallets` | `pay` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/users/{userId}/wallets/{currencyCode}/payment` | Pays with a user wallet by currency code and client platform |
| `wallets` | `reset-platform-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/platforms/{platform}/wallet/config/reset` | Resets the platform wallet config to defaults |
| `wallets` | `update-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/wallet/config` | Updates the wallet configuration for a namespace |
| `wallets` | `update-platform-config` | admin | v1 | PUT | `/platform/admin/namespaces/{namespace}/platforms/{platform}/wallet/config` | Updates the platform wallet config |

## reporting

- Spec name: `reporting`
- Resources: 6
- Operations: 35

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `config` | `get` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/configurations` | Retrieves the reporting configuration for a namespace |
| `config` | `upsert` | admin | v1 | POST | `/reporting/v1/admin/namespaces/{namespace}/configurations` | Creates or updates the reporting configuration for a namespace |
| `extensions` | `create-action` | admin | v1 | POST | `/reporting/v1/admin/extensionActions` | Creates an auto moderation action |
| `extensions` | `create-category` | admin | v1 | POST | `/reporting/v1/admin/extensionCategories` | Creates an extension category |
| `extensions` | `list-actions` | admin | v1 | GET | `/reporting/v1/admin/extensionActions` | Retrieves all auto moderation actions |
| `extensions` | `list-categories` | admin | v1 | GET | `/reporting/v1/admin/extensionCategories` | Retrieves all extension categories |
| `reasons` | `create` | admin | v1 | POST | `/reporting/v1/admin/namespaces/{namespace}/reasons` | Creates a report reason |
| `reasons` | `create-group` | admin | v1 | POST | `/reporting/v1/admin/namespaces/{namespace}/reasonGroups` | Creates a reason group |
| `reasons` | `delete` | admin | v1 | DELETE | `/reporting/v1/admin/namespaces/{namespace}/reasons/{reasonId}` | Deletes a report reason |
| `reasons` | `delete-group` | admin | v1 | DELETE | `/reporting/v1/admin/namespaces/{namespace}/reasonGroups/{groupId}` | Deletes a reason group |
| `reasons` | `get` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/reasons/{reasonId}` | Retrieves a single report reason |
| `reasons` | `get-group` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/reasonGroups/{groupId}` | Retrieves a reason group by ID |
| `reasons` | `list` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/reasons` | Lists reasons in a namespace |
| `reasons` | `list` | public | v1 | GET | `/reporting/v1/public/namespaces/{namespace}/reasons` | Lists reasons for a namespace |
| `reasons` | `list-all` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/reasons/all` | Retrieves all reasons without pagination |
| `reasons` | `list-groups` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/reasonGroups` | Lists reason groups in a namespace |
| `reasons` | `list-groups` | public | v1 | GET | `/reporting/v1/public/namespaces/{namespace}/reasonGroups` | Lists reason groups for a namespace |
| `reasons` | `list-unused` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/reasons/unused` | Lists reasons not used by any moderation rule |
| `reasons` | `update` | admin | v1 | PATCH | `/reporting/v1/admin/namespaces/{namespace}/reasons/{reasonId}` | Updates a report reason |
| `reasons` | `update-group` | admin | v1 | PATCH | `/reporting/v1/admin/namespaces/{namespace}/reasonGroups/{groupId}` | Updates a reason group |
| `reports` | `create` | admin | v1 | POST | `/reporting/v1/admin/namespaces/{namespace}/reports` | Submits a report as an admin |
| `reports` | `create` | public | v1 | POST | `/reporting/v1/public/namespaces/{namespace}/reports` | Submits a report |
| `reports` | `list` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/reports` | Lists submitted reports for a namespace |
| `rules` | `create` | admin | v1 | POST | `/reporting/v1/admin/namespaces/{namespace}/rule` | Creates an auto moderation rule |
| `rules` | `delete` | admin | v1 | DELETE | `/reporting/v1/admin/namespaces/{namespace}/rule/{ruleId}` | Deletes an auto moderation rule |
| `rules` | `get` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/rules/{ruleId}` | Retrieves an auto moderation rule |
| `rules` | `list` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/rules` | Lists auto moderation rules for a namespace |
| `rules` | `update` | admin | v1 | PUT | `/reporting/v1/admin/namespaces/{namespace}/rule/{ruleId}` | Updates an auto moderation rule |
| `rules` | `update-status` | admin | v1 | PUT | `/reporting/v1/admin/namespaces/{namespace}/rule/{ruleId}/status` | Enables or disables an auto moderation rule |
| `tickets` | `delete` | admin | v1 | DELETE | `/reporting/v1/admin/namespaces/{namespace}/tickets/{ticketId}` | Deletes a report ticket |
| `tickets` | `get` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/tickets/{ticketId}` | Retrieves a single report ticket |
| `tickets` | `get-statistics` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/tickets/statistic` | Retrieves ticket statistics |
| `tickets` | `list` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/tickets` | Lists report tickets for a namespace |
| `tickets` | `list-reports` | admin | v1 | GET | `/reporting/v1/admin/namespaces/{namespace}/tickets/{ticketId}/reports` | Lists reports for a ticket |
| `tickets` | `update-resolution` | admin | v1 | POST | `/reporting/v1/admin/namespaces/{namespace}/tickets/{ticketId}/resolutions` | Updates a ticket's resolution status |

## season-pass

- Spec name: `seasonpass`
- Resources: 6
- Operations: 45

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `passes` | `create` | admin | v1 | POST | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/passes` | Creates a pass for a season |
| `passes` | `delete` | admin | v1 | DELETE | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/passes/{code}` | Deletes a pass permanently |
| `passes` | `get` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/passes/{code}` | Retrieves a pass by code |
| `passes` | `grant` | admin | v1 | POST | `/seasonpass/admin/namespaces/{namespace}/users/{userId}/seasons/current/passes` | Grants a pass to a user |
| `passes` | `list` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/passes` | Lists all passes for a season |
| `passes` | `update` | admin | v1 | PATCH | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/passes/{code}` | Updates a pass |
| `progress` | `bulk-claim-rewards` | public | v1 | POST | `/seasonpass/public/namespaces/{namespace}/users/{userId}/seasons/current/rewards/bulk` | Claims all remaining tier rewards |
| `progress` | `check-season-ownership` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/users/{userId}/seasons/current/passes/ownership/any` | Checks ownership of any season pass |
| `progress` | `claim-rewards` | public | v1 | POST | `/seasonpass/public/namespaces/{namespace}/users/{userId}/seasons/current/rewards` | Claims a tier reward |
| `progress` | `get` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/users/{userId}/seasons/current/progression` | Retrieves the current user's season progression |
| `progress` | `get-bulk` | admin | v1 | POST | `/seasonpass/admin/namespaces/{namespace}/seasons/current/users/bulk/progression` | Retrieves current season progression for a batch of users |
| `progress` | `get-current-season` | public | v1 | GET | `/seasonpass/public/namespaces/{namespace}/users/{userId}/seasons/current/data` | Retrieves the current user's season data |
| `progress` | `get-experience-history` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/users/{userId}/seasons/exp/history` | Lists a user's season experience acquisition history |
| `progress` | `get-season` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/users/{userId}/seasons/{seasonId}/data` | Retrieves a user's data for a specific season |
| `progress` | `get-season` | public | v1 | GET | `/seasonpass/public/namespaces/{namespace}/users/{userId}/seasons/{seasonId}/data` | Retrieves a user's data for a specific season |
| `progress` | `grant-experience` | admin | v1 | POST | `/seasonpass/admin/namespaces/{namespace}/users/{userId}/seasons/current/exp` | Grants experience points to a user |
| `progress` | `list-experience-history-tags` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/users/{userId}/seasons/exp/history/tags` | Lists reason tags for a user's experience history |
| `progress` | `list-participated-seasons` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/users/{userId}/seasons` | Retrieves a user's participated season data |
| `progress` | `reset` | admin | v1 | DELETE | `/seasonpass/admin/namespaces/{namespace}/users/{userId}/seasons/current/reset` | Resets user data in the current season |
| `rewards` | `create` | admin | v1 | POST | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/rewards` | Creates a reward for a season |
| `rewards` | `delete` | admin | v1 | DELETE | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/rewards/{code}` | Deletes a reward permanently |
| `rewards` | `get` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/rewards/{code}` | Retrieves a reward by code |
| `rewards` | `list` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/rewards` | Lists all rewards for a season |
| `rewards` | `update` | admin | v1 | PATCH | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/rewards/{code}` | Updates a reward |
| `seasons` | `check-purchaseable` | admin | v1 | POST | `/seasonpass/admin/namespaces/{namespace}/users/{userId}/seasons/current/purchasable` | Checks if a pass or tier is purchasable |
| `seasons` | `clone` | admin | v1 | POST | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/clone` | Clones a season |
| `seasons` | `create` | admin | v1 | POST | `/seasonpass/admin/namespaces/{namespace}/seasons` | Creates a season in the namespace |
| `seasons` | `delete` | admin | v1 | DELETE | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}` | Deletes a season permanently |
| `seasons` | `export` | admin | v1 | GET | `/seasonpass/admin/namespace/{namespace}/export` | Exports season service data |
| `seasons` | `get` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}` | Retrieves a season by ID |
| `seasons` | `get-current` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/seasons/current` | Retrieves the current published season summary |
| `seasons` | `get-current` | public | v1 | GET | `/seasonpass/public/namespaces/{namespace}/seasons/current` | Retrieves the current published season |
| `seasons` | `get-full` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/full` | Retrieves the full content of a season |
| `seasons` | `list` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/seasons` | Lists seasons in the namespace |
| `seasons` | `publish` | admin | v1 | PUT | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/publish` | Publishes a season |
| `seasons` | `retire` | admin | v1 | PUT | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/retire` | Retires a published season |
| `seasons` | `unpublish` | admin | v1 | PUT | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/unpublish` | Unpublishes a season |
| `seasons` | `update` | admin | v1 | PATCH | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}` | Updates a season |
| `tiers` | `create` | admin | v1 | POST | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/tiers` | Creates tiers for a season |
| `tiers` | `delete` | admin | v1 | DELETE | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/tiers/{id}` | Deletes a tier permanently |
| `tiers` | `grant` | admin | v1 | POST | `/seasonpass/admin/namespaces/{namespace}/users/{userId}/seasons/current/tiers` | Grants a tier to a user |
| `tiers` | `list` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/tiers` | Lists all tiers for a season |
| `tiers` | `reorder` | admin | v1 | PUT | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/tiers/{id}/reorder` | Moves a tier to a new position |
| `tiers` | `update` | admin | v1 | PUT | `/seasonpass/admin/namespaces/{namespace}/seasons/{seasonId}/tiers/{id}` | Updates a tier |
| `utilities` | `list-item-references` | admin | v1 | GET | `/seasonpass/admin/namespaces/{namespace}/seasons/item/references` | Retrieves season pass item references from the ecommerce store |

## session

- Spec name: `session`
- Resources: 9
- Operations: 88

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `alerts` | `create` | admin | v1 | POST | `/session/v1/admin/namespaces/{namespace}/alerts-configuration` | Creates an alerts configuration for the namespace |
| `alerts` | `delete` | admin | v1 | DELETE | `/session/v1/admin/namespaces/{namespace}/alerts-configuration` | Deletes the alerts configuration for the namespace |
| `alerts` | `get` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/alerts-configuration` | Retrieves the alerts configuration for the namespace |
| `alerts` | `update` | admin | v1 | PUT | `/session/v1/admin/namespaces/{namespace}/alerts-configuration` | Updates the alerts configuration for the namespace |
| `config` | `delete` | admin | v1 | DELETE | `/session/v1/admin/global-configurations` | Deletes global configuration data |
| `config` | `get-log` | admin | v1 | GET | `/session/v1/admin/config/log` | Retrieves the log configuration |
| `config` | `list` | admin | v1 | GET | `/session/v1/admin/global-configurations` | Retrieves global configuration data |
| `config` | `list-environment-variables` | admin | v1 | GET | `/session/v1/admin/environment-variables` | Lists environment variables for the session service |
| `config` | `update` | admin | v1 | PUT | `/session/v1/admin/global-configurations` | Creates or updates global configuration data |
| `config` | `update-log` | admin | v1 | PATCH | `/session/v1/admin/config/log` | Updates the log configuration |
| `game-sessions` | `bulk-delete` | admin | v1 | DELETE | `/session/v1/admin/namespaces/{namespace}/gamesessions/bulk` | Deletes game sessions in bulk |
| `game-sessions` | `cancel-invitation` | public | v1 | DELETE | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}/users/{userId}/cancel` | Cancels a game session invitation |
| `game-sessions` | `create` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/gamesession` | Creates a game session |
| `game-sessions` | `delete` | public | v1 | DELETE | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}` | Deletes a game session |
| `game-sessions` | `generate-code` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}/code` | Generates an invite code for a game session |
| `game-sessions` | `get` | public | v1 | GET | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}` | Retrieves a game session |
| `game-sessions` | `get-active` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/configurations/{name}/memberactivesession/{userId}` | Retrieves the active session for a member in a configuration |
| `game-sessions` | `get-dedicated-server-secret` | public | v1 | GET | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}/secret` | Retrieves the server secret for a game session |
| `game-sessions` | `get-pod` | public | v1 | GET | `/session/v1/public/namespaces/{namespace}/gamesessions/servers/{podName}` | Retrieves a game session by server pod name |
| `game-sessions` | `invite` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}/invite` | Invites a user to a game session |
| `game-sessions` | `join` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}/join` | Joins a game session |
| `game-sessions` | `join-by-code` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/gamesessions/join/code` | Joins a game session using an invite code |
| `game-sessions` | `kick` | admin | v1 | DELETE | `/session/v1/admin/namespaces/{namespace}/gamesessions/{sessionId}/members/{memberId}/kick` | Kicks a member from a game session |
| `game-sessions` | `kick` | public | v1 | DELETE | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}/members/{memberId}/kick` | Kicks a member from a game session (leader only) |
| `game-sessions` | `leave` | public | v1 | DELETE | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}/leave` | Leaves a game session |
| `game-sessions` | `list` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/gamesessions` | Lists all game sessions in the namespace |
| `game-sessions` | `list-filtered` | admin | v1 | POST | `/session/v1/admin/namespaces/{namespace}/gamesessions` | Queries game sessions using attribute filters |
| `game-sessions` | `list-filtered` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/gamesessions` | Queries game sessions using attribute filters |
| `game-sessions` | `list-my` | public | v1 | GET | `/session/v1/public/namespaces/{namespace}/users/me/gamesessions` | Retrieves a list of the current user's game sessions |
| `game-sessions` | `list-native` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/native-sessions` | Lists native sessions in the namespace |
| `game-sessions` | `reconcile-active-count` | admin | v1 | POST | `/session/v1/admin/namespaces/{namespace}/configurations/{name}/reconcile` | Reconciles the max active session count for a configuration template |
| `game-sessions` | `reject-invitation` | public | v1 | DELETE | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}/reject` | Rejects a game session invitation |
| `game-sessions` | `revoke-code` | public | v1 | DELETE | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}/code` | Revokes the game session invite code |
| `game-sessions` | `set` | public | v1 | PUT | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}` | Updates a game session |
| `game-sessions` | `set-dedicated-server-ready` | admin | v1 | PUT | `/session/v1/admin/namespaces/{namespace}/gamesessions/{sessionId}/ds` | Sets the DS as ready to accept connections |
| `game-sessions` | `set-leader` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}/leader` | Promotes a member to game session leader |
| `game-sessions` | `update` | public | v1 | PATCH | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}` | Applies partial field updates to the specified game session |
| `game-sessions` | `update-backfill` | public | v1 | PUT | `/session/v1/public/namespaces/{namespace}/gamesessions/{sessionId}/backfill` | Updates the backfill ticket ID for a game session |
| `game-sessions` | `update-dedicated-server` | admin | v1 | PUT | `/session/v1/admin/namespaces/{namespace}/gamesessions/{sessionId}/dsinformation` | Updates DS information for a game session asynchronous process |
| `game-sessions` | `update-member` | admin | v1 | PUT | `/session/v1/admin/namespaces/{namespace}/gamesessions/{sessionId}/members/{memberId}/status/{statusType}` | Updates the status of a game session member |
| `parties` | `bulk-delete` | admin | v1 | DELETE | `/session/v1/admin/namespaces/{namespace}/parties/bulk` | Deletes parties in bulk |
| `parties` | `cancel-invitation` | public | v1 | DELETE | `/session/v1/public/namespaces/{namespace}/parties/{partyId}/users/{userId}/cancel` | Cancels a pending invitation for the specified user |
| `parties` | `create` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/party` | Creates a new party and adds the requester as leader |
| `parties` | `generate-code` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/parties/{partyId}/code` | Generates an invite code for a party |
| `parties` | `get` | public | v1 | GET | `/session/v1/public/namespaces/{namespace}/parties/{partyId}` | Retrieves party details |
| `parties` | `invite` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/parties/{partyId}/invite` | Invites a user to a party |
| `parties` | `join` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/parties/{partyId}/users/me/join` | Joins a party by invitation or open joinability |
| `parties` | `join-by-code` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/parties/users/me/join/code` | Joins a party using an invite code |
| `parties` | `kick` | public | v1 | DELETE | `/session/v1/public/namespaces/{namespace}/parties/{partyId}/users/{userId}/kick` | Kicks the specified player from the party |
| `parties` | `leave` | public | v1 | DELETE | `/session/v1/public/namespaces/{namespace}/parties/{partyId}/users/me/leave` | Leaves the specified party |
| `parties` | `list-my` | public | v1 | GET | `/session/v1/public/namespaces/{namespace}/users/me/parties` | Retrieves a list of the current user's parties |
| `parties` | `reject-invitation` | public | v1 | DELETE | `/session/v1/public/namespaces/{namespace}/parties/{partyId}/users/me/reject` | Rejects an invitation to the specified party |
| `parties` | `revoke-code` | public | v1 | DELETE | `/session/v1/public/namespaces/{namespace}/parties/{partyId}/code` | Revokes the party invite code |
| `parties` | `search` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/parties` | Queries parties in the namespace |
| `parties` | `set` | public | v1 | PUT | `/session/v1/public/namespaces/{namespace}/parties/{partyId}` | Updates a party |
| `parties` | `set-leader` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/parties/{partyId}/leader` | Promotes a new party leader for the specified party |
| `parties` | `sync-native` | admin | v1 | POST | `/session/v1/admin/namespaces/{namespace}/users/{userId}/native-sync` | Syncs the user's active party session to the native platform |
| `parties` | `update` | public | v1 | PATCH | `/session/v1/public/namespaces/{namespace}/parties/{partyId}` | Applies partial field updates to the specified party |
| `platform-credentials` | `delete` | admin | v1 | DELETE | `/session/v1/admin/namespaces/{namespace}/platform-credentials/{platformId}` | Deletes platform credentials for the specified platform |
| `platform-credentials` | `delete-all` | admin | v1 | DELETE | `/session/v1/admin/namespaces/{namespace}/platform-credentials` | Deletes all platform credentials for the namespace |
| `platform-credentials` | `list` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/platform-credentials` | Retrieves platform credentials for native session sync |
| `platform-credentials` | `sync` | admin | v1 | PUT | `/session/v1/admin/namespaces/{namespace}/platform-credentials/{platformId}/sync` | Syncs platform credentials from the platform service |
| `platform-credentials` | `update` | admin | v1 | PUT | `/session/v1/admin/namespaces/{namespace}/platform-credentials` | Updates platform credentials for native session sync |
| `platform-credentials` | `upload` | admin | v1 | PUT | `/session/v1/admin/namespaces/{namespace}/platform-credentials/{platformId}/upload` | Uploads certificates for the Xbox platform |
| `recent-users` | `get` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/recent-player` | Queries recent players for a given user |
| `recent-users` | `get` | public | v1 | GET | `/session/v1/public/namespaces/{namespace}/recent-player` | Retrieves a list of the user's recent players |
| `recent-users` | `get-team` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/recent-team-player` | Queries recent team players for a given user |
| `recent-users` | `get-team` | public | v1 | GET | `/session/v1/public/namespaces/{namespace}/recent-team-player` | Retrieves a list of the user's recent players from the same team |
| `storage` | `delete` | admin | v1 | DELETE | `/session/v1/admin/namespaces/{namespace}/sessions/{sessionId}/storage` | Deletes session storage for a game session |
| `storage` | `get` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/sessions/{sessionId}/storage` | Retrieves session storage for a game session |
| `storage` | `get-party` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/parties/{partyId}/storage` | Retrieves party session storage |
| `storage` | `get-party` | public | v1 | GET | `/session/v1/public/namespaces/{namespace}/parties/{partyId}/storage` | Retrieves the party session storage data |
| `storage` | `get-user` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/sessions/{sessionId}/storage/users/{userId}` | Retrieves session storage for a user |
| `storage` | `upsert` | public | v1 | PATCH | `/session/v1/public/namespaces/{namespace}/sessions/{sessionId}/storage/users/{userId}` | Updates or inserts session storage data for the specified user |
| `storage` | `upsert-leader` | public | v1 | PATCH | `/session/v1/public/namespaces/{namespace}/sessions/{sessionId}/storage/leader` | Updates or inserts session storage data for the session leader |
| `storage` | `upsert-party` | public | v1 | PATCH | `/session/v1/public/namespaces/{namespace}/parties/{partyId}/storage/users/{userId}` | Updates or inserts party session storage data for the specified user |
| `storage` | `upsert-party-reserved` | public | v1 | PATCH | `/session/v1/public/namespaces/{namespace}/parties/{partyId}/storage/users/{userId}/reserved` | Updates or inserts reserved party session storage data for the specified user |
| `templates` | `create` | admin | v1 | POST | `/session/v1/admin/namespaces/{namespace}/configuration` | Creates a configuration template for parties and game sessions |
| `templates` | `delete` | admin | v1 | DELETE | `/session/v1/admin/namespaces/{namespace}/configurations/{name}` | Deletes the named configuration template |
| `templates` | `get` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/configurations/{name}` | Retrieves a configuration template by name |
| `templates` | `list` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/configurations` | Retrieves configuration templates in the namespace |
| `templates` | `update` | admin | v1 | PUT | `/session/v1/admin/namespaces/{namespace}/configurations/{name}` | Updates the named configuration template |
| `users` | `bulk-get-attributes` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/users/attributes` | Retrieves player attributes for a list of users |
| `users` | `bulk-get-platforms` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/users/bulk/platform` | Retrieves the current platform for a list of players |
| `users` | `create-my-attributes` | public | v1 | POST | `/session/v1/public/namespaces/{namespace}/users/me/attributes` | Saves the current user's player attributes |
| `users` | `delete-attributes` | public | v1 | DELETE | `/session/v1/public/namespaces/{namespace}/users/me/attributes` | Removes the current user's player attributes |
| `users` | `list-attributes` | admin | v1 | GET | `/session/v1/admin/namespaces/{namespace}/users/{userId}/attributes` | Retrieves player attributes for a user |
| `users` | `list-my-attributes` | public | v1 | GET | `/session/v1/public/namespaces/{namespace}/users/me/attributes` | Retrieves the current user's player attributes |

## session-history

- Spec name: `sessionhistory`
- Resources: 1
- Operations: 2

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `xray-tickets` | `bulk-create` | admin | v2 | POST | `/sessionhistory/v2/admin/namespaces/{namespace}/xray/tickets/bulk` | Creates ticket observability records in bulk |
| `xray-tickets` | `create` | admin | v2 | POST | `/sessionhistory/v2/admin/namespaces/{namespace}/xray/tickets` | Creates a ticket observability record |

## social

- Spec name: `social`
- Resources: 5
- Operations: 74

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `global-stat-values` | `get` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/globalstatitems/{statCode}` | Retrieves a global stat item by stat code |
| `global-stat-values` | `get` | public | v1 | GET | `/social/v1/public/namespaces/{namespace}/globalstatitems/{statCode}` | Retrieves a global stat item by stat code |
| `global-stat-values` | `list` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/globalstatitems` | Lists global stat items |
| `global-stat-values` | `list` | public | v1 | GET | `/social/v1/public/namespaces/{namespace}/globalstatitems` | Lists global stat items |
| `stat-cycles` | `bulk-add` | admin | v1 | POST | `/social/v1/admin/namespaces/{namespace}/statCycles/{cycleId}/stats/add/bulk` | Adds a stat cycle to stats in bulk |
| `stat-cycles` | `bulk-get` | admin | v1 | POST | `/social/v1/admin/namespaces/{namespace}/statCycles/bulk` | Retrieves stat cycles in bulk |
| `stat-cycles` | `bulk-get` | public | v1 | POST | `/social/v1/public/namespaces/{namespace}/statCycles/bulk` | Retrieves stat cycles in bulk |
| `stat-cycles` | `create` | admin | v1 | POST | `/social/v1/admin/namespaces/{namespace}/statCycles` | Creates a stat cycle |
| `stat-cycles` | `delete` | admin | v1 | DELETE | `/social/v1/admin/namespaces/{namespace}/statCycles/{cycleId}` | Deletes stat cycle |
| `stat-cycles` | `export` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/statCycles/export` | Exports all stat cycle configurations |
| `stat-cycles` | `export` | public | v1 | GET | `/social/v1/public/namespaces/{namespace}/statCycles/{cycleId}` | Retrieves a stat cycle |
| `stat-cycles` | `get` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/statCycles/{cycleId}` | Retrieves a stat cycle |
| `stat-cycles` | `import` | admin | v1 | POST | `/social/v1/admin/namespaces/{namespace}/statCycles/import` | Imports stat cycle configurations |
| `stat-cycles` | `list` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/statCycles` | Lists stat cycles |
| `stat-cycles` | `list` | public | v1 | GET | `/social/v1/public/namespaces/{namespace}/statCycles` | Lists stat cycles |
| `stat-cycles` | `stop` | admin | v1 | PUT | `/social/v1/admin/namespaces/{namespace}/statCycles/{cycleId}/stop` | Stops a stat cycle |
| `stat-cycles` | `update` | admin | v1 | PUT | `/social/v1/admin/namespaces/{namespace}/statCycles/{cycleId}` | Updates a stat cycle |
| `stat-definitions` | `create` | admin | v1 | POST | `/social/v1/admin/namespaces/{namespace}/stats` | Creates a stat |
| `stat-definitions` | `create` | public | v1 | POST | `/social/v1/public/namespaces/{namespace}/stats` | Creates a stat |
| `stat-definitions` | `delete` | admin | v1 | DELETE | `/social/v1/admin/namespaces/{namespace}/stats/{statCode}` | Deletes stat |
| `stat-definitions` | `delete-tied` | admin | v1 | DELETE | `/social/v1/admin/namespaces/{namespace}/stats/{statCode}/tied` | Deletes tied stat |
| `stat-definitions` | `export` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/stats/export` | Exports all stat configurations |
| `stat-definitions` | `get` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/stats/{statCode}` | Retrieves a stat by stat code |
| `stat-definitions` | `import` | admin | v1 | POST | `/social/v1/admin/namespaces/{namespace}/stats/import` | Imports stat configurations |
| `stat-definitions` | `list` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/stats` | Lists stats |
| `stat-definitions` | `search` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/stats/search` | Searches stats by keyword |
| `stat-definitions` | `update` | admin | v1 | PATCH | `/social/v1/admin/namespaces/{namespace}/stats/{statCode}` | Updates a stat |
| `user-stat-cycles` | `list` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/users/{userId}/statCycles/{cycleId}/statCycleitems` | Lists a user's stat cycle items for a stat cycle |
| `user-stat-cycles` | `list` | public | v1 | GET | `/social/v1/public/namespaces/{namespace}/users/{userId}/statCycles/{cycleId}/statCycleitems` | Lists a user's stat cycle items for a cycle |
| `user-stat-cycles` | `list-my` | public | v1 | GET | `/social/v1/public/namespaces/{namespace}/users/me/statCycles/{cycleId}/statCycleitems` | Lists the current user's stat cycle items for a cycle |
| `user-stat-values` | `bulk-create` | admin | v1 | POST | `/social/v1/admin/namespaces/{namespace}/users/{userId}/statitems/bulk` | Creates stat items for a user in bulk |
| `user-stat-values` | `bulk-create` | public | v1 | POST | `/social/v1/public/namespaces/{namespace}/users/{userId}/statitems/bulk` | Creates stat items for a user in bulk |
| `user-stat-values` | `bulk-get` | admin | v2 | POST | `/social/v2/admin/namespaces/{namespace}/users/{userId}/statitems/value/bulk/getOrDefault` | Retrieves a user's stat item values in bulk by stat code |
| `user-stat-values` | `bulk-get` | public | v1 | GET | `/social/v1/public/namespaces/{namespace}/users/{userId}/statitems/value/bulk` | Lists a user's stat item values in bulk |
| `user-stat-values` | `bulk-get` | public | v2 | GET | `/social/v2/public/namespaces/{namespace}/users/{userId}/statitems/value/bulk` | Lists a user's stat item values in bulk |
| `user-stat-values` | `bulk-get-by-code` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/statitems/value/bulk/getOrDefault` | Retrieves stat item values for users, returning defaults for missing items |
| `user-stat-values` | `bulk-get-by-code` | admin | v2 | GET | `/social/v2/admin/namespaces/{namespace}/statitems/value/bulk/getOrDefault` | Retrieves stat item values for users in bulk by stat code |
| `user-stat-values` | `bulk-get-for-users` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/statitems/bulk` | Retrieves stat item values for users in bulk |
| `user-stat-values` | `bulk-get-for-users` | public | v1 | GET | `/social/v1/public/namespaces/{namespace}/statitems/bulk` | Retrieves stat item values for users in bulk |
| `user-stat-values` | `bulk-get-my` | public | v1 | GET | `/social/v1/public/namespaces/{namespace}/users/me/statitems/value/bulk` | Lists the current user's stat item values in bulk |
| `user-stat-values` | `bulk-reset` | admin | v1 | PUT | `/social/v1/admin/namespaces/{namespace}/users/{userId}/statitems/value/reset/bulk` | Resets a user's stat item values to defaults in bulk |
| `user-stat-values` | `bulk-reset` | admin | v2 | PUT | `/social/v2/admin/namespaces/{namespace}/users/{userId}/statitems/value/reset/bulk` | Resets a user's stat item values to defaults in bulk |
| `user-stat-values` | `bulk-reset` | public | v1 | PUT | `/social/v1/public/namespaces/{namespace}/users/{userId}/statitems/value/reset/bulk` | Resets a user's stat item values to defaults in bulk |
| `user-stat-values` | `bulk-reset-for-users` | admin | v1 | PUT | `/social/v1/admin/namespaces/{namespace}/statitems/value/reset/bulk` | Resets stat item values to defaults for users in bulk |
| `user-stat-values` | `bulk-reset-for-users` | public | v1 | PUT | `/social/v1/public/namespaces/{namespace}/statitems/value/reset/bulk` | Resets stat item values for users in bulk |
| `user-stat-values` | `bulk-set` | admin | v1 | PUT | `/social/v1/admin/namespaces/{namespace}/users/{userId}/statitems/value/bulk` | Replaces a user's stat item values in bulk |
| `user-stat-values` | `bulk-set` | public | v1 | PUT | `/social/v1/public/namespaces/{namespace}/users/{userId}/statitems/value/bulk` | Replaces a user's stat item values in bulk |
| `user-stat-values` | `bulk-set-for-users` | admin | v1 | PUT | `/social/v1/admin/namespaces/{namespace}/statitems/value/bulk` | Replaces stat item values for users in bulk |
| `user-stat-values` | `bulk-set-for-users` | public | v1 | PUT | `/social/v1/public/namespaces/{namespace}/statitems/value/bulk` | Replaces stat item values for users in bulk |
| `user-stat-values` | `bulk-update` | admin | v2 | PUT | `/social/v2/admin/namespaces/{namespace}/users/{userId}/statitems/value/bulk` | Updates a user's stat item values in bulk |
| `user-stat-values` | `bulk-update` | public | v2 | PUT | `/social/v2/public/namespaces/{namespace}/users/{userId}/statitems/value/bulk` | Updates a user's stat item values in bulk |
| `user-stat-values` | `bulk-update-for-users` | admin | v1 | PATCH | `/social/v1/admin/namespaces/{namespace}/statitems/value/bulk` | Updates stat item values for users in bulk |
| `user-stat-values` | `bulk-update-for-users` | admin | v2 | PUT | `/social/v2/admin/namespaces/{namespace}/statitems/value/bulk` | Updates stat item values for users in bulk |
| `user-stat-values` | `bulk-update-for-users` | public | v1 | PATCH | `/social/v1/public/namespaces/{namespace}/statitems/value/bulk` | Updates stat item values for users in bulk |
| `user-stat-values` | `bulk-update-for-users` | public | v2 | PUT | `/social/v2/public/namespaces/{namespace}/statitems/value/bulk` | Updates stat item values for users in bulk |
| `user-stat-values` | `bulk-update-user` | admin | v1 | PATCH | `/social/v1/admin/namespaces/{namespace}/users/{userId}/statitems/value/bulk` | Updates a user's stat item values in bulk |
| `user-stat-values` | `bulk-update-user` | public | v1 | PATCH | `/social/v1/public/namespaces/{namespace}/users/{userId}/statitems/value/bulk` | Updates a user's stat item values in bulk |
| `user-stat-values` | `create` | admin | v1 | POST | `/social/v1/admin/namespaces/{namespace}/users/{userId}/stats/{statCode}/statitems` | Creates a stat item for a user |
| `user-stat-values` | `create` | public | v1 | POST | `/social/v1/public/namespaces/{namespace}/users/{userId}/stats/{statCode}/statitems` | Creates a stat item for a user |
| `user-stat-values` | `delete` | admin | v1 | DELETE | `/social/v1/admin/namespaces/{namespace}/users/{userId}/stats/{statCode}/statitems` | Deletes a user's stat items |
| `user-stat-values` | `delete` | admin | v2 | DELETE | `/social/v2/admin/namespaces/{namespace}/users/{userId}/stats/{statCode}/statitems` | Deletes stat items for a user |
| `user-stat-values` | `delete` | public | v1 | DELETE | `/social/v1/public/namespaces/{namespace}/users/{userId}/stats/{statCode}/statitems` | Deletes a user's stat item |
| `user-stat-values` | `get` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/stats/{statCode}/statitems` | Retrieves stat item values for users by stat code |
| `user-stat-values` | `list` | admin | v1 | GET | `/social/v1/admin/namespaces/{namespace}/users/{userId}/statitems` | Lists a user's stat items |
| `user-stat-values` | `list` | admin | v2 | GET | `/social/v2/admin/namespaces/{namespace}/users/{userId}/statitems/value/bulk` | Lists a user's stat item values (legacy) |
| `user-stat-values` | `list` | public | v1 | GET | `/social/v1/public/namespaces/{namespace}/users/{userId}/statitems` | Lists a user's stat items |
| `user-stat-values` | `list-my` | public | v1 | GET | `/social/v1/public/namespaces/{namespace}/users/me/statitems` | Lists the current user's stat items |
| `user-stat-values` | `reset` | admin | v1 | PUT | `/social/v1/admin/namespaces/{namespace}/users/{userId}/stats/{statCode}/statitems/value/reset` | Resets a user's stat item value to its default |
| `user-stat-values` | `reset` | public | v1 | PUT | `/social/v1/public/namespaces/{namespace}/users/{userId}/stats/{statCode}/statitems/value/reset` | Resets a user's stat item to its default value |
| `user-stat-values` | `set` | public | v1 | PUT | `/social/v1/public/namespaces/{namespace}/users/{userId}/stats/{statCode}/statitems/value` | Replaces a user's stat item value |
| `user-stat-values` | `update` | admin | v1 | PATCH | `/social/v1/admin/namespaces/{namespace}/users/{userId}/stats/{statCode}/statitems/value` | Updates a user's stat item value |
| `user-stat-values` | `update` | admin | v2 | PUT | `/social/v2/admin/namespaces/{namespace}/users/{userId}/stats/{statCode}/statitems/value` | Updates a user's stat item value |
| `user-stat-values` | `update` | public | v1 | PATCH | `/social/v1/public/namespaces/{namespace}/users/{userId}/stats/{statCode}/statitems/value` | Updates a user's stat item value |
| `user-stat-values` | `update` | public | v2 | PUT | `/social/v2/public/namespaces/{namespace}/users/{userId}/stats/{statCode}/statitems/value` | Updates a user's stat item value |

## ugc

- Spec name: `ugc`
- Resources: 10
- Operations: 147

| Resource | Method | Scope | Version | HTTP | Path | Summary |
|----------|--------|-------|---------|------|------|---------|
| `channels` | `create` | admin | v1 | POST | `/ugc/v1/admin/namespaces/{namespace}/channels` | Creates a channel in the namespace |
| `channels` | `create` | public | v1 | POST | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/channels` | Creates a channel for a user |
| `channels` | `delete` | admin | v1 | DELETE | `/ugc/v1/admin/namespaces/{namespace}/channels/{channelId}` | Deletes a channel from the namespace |
| `channels` | `delete` | public | v1 | DELETE | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/channels/{channelId}` | Deletes a channel for a user |
| `channels` | `delete-user` | admin | v1 | DELETE | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/channels/{channelId}` | Deletes a channel for a user |
| `channels` | `list` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/channels` | Lists channels in the namespace |
| `channels` | `list` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/channels` | Lists channels for a user |
| `channels` | `list-user` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/channels` | Lists channels for a user |
| `channels` | `update` | admin | v1 | PUT | `/ugc/v1/admin/namespaces/{namespace}/channels/{channelId}` | Updates a channel in the namespace |
| `channels` | `update` | public | v1 | PUT | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/channels/{channelId}` | Updates a user's channel |
| `channels` | `update-user` | admin | v1 | PUT | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/channels/{channelId}` | Updates a user's channel |
| `config` | `list` | admin | v2 | GET | `/ugc/v2/admin/namespaces/{namespace}/configs` | Retrieves configuration settings paginated |
| `config` | `update` | admin | v2 | PATCH | `/ugc/v2/admin/namespaces/{namespace}/configs/{key}` | Updates a configuration value |
| `content` | `bulk-get` | admin | v1 | POST | `/ugc/v1/admin/namespaces/{namespace}/contents/bulk` | Retrieves content entries by ID |
| `content` | `bulk-get` | admin | v2 | POST | `/ugc/v2/admin/namespaces/{namespace}/contents/bulk` | Retrieves content entries by ID |
| `content` | `bulk-get` | public | v1 | POST | `/ugc/v1/public/namespaces/{namespace}/contents/bulk` | Retrieves content entries by ID |
| `content` | `bulk-get` | public | v2 | POST | `/ugc/v2/public/namespaces/{namespace}/contents/bulk` | Retrieves content entries by ID |
| `content` | `bulk-get-by-share-code` | admin | v1 | POST | `/ugc/v1/admin/namespaces/{namespace}/contents/sharecodes/bulk` | Retrieves content entries by share code |
| `content` | `bulk-get-by-share-code` | admin | v2 | POST | `/ugc/v2/admin/namespaces/{namespace}/contents/sharecodes/bulk` | Retrieves content entries by share code |
| `content` | `bulk-get-by-share-code` | public | v1 | POST | `/ugc/v1/public/namespaces/{namespace}/contents/sharecodes/bulk` | Retrieves content entries by share code |
| `content` | `bulk-get-by-share-code` | public | v2 | POST | `/ugc/v2/public/namespaces/{namespace}/contents/sharecodes/bulk` | Retrieves content entries by share code |
| `content` | `copy` | admin | v2 | POST | `/ugc/v2/admin/namespaces/{namespace}/channels/{channelId}/contents/{contentId}/copy` | Copies a content entry to another channel |
| `content` | `create` | admin | v2 | POST | `/ugc/v2/admin/namespaces/{namespace}/channels/{channelId}/contents` | Creates official content in a channel |
| `content` | `create` | public | v2 | POST | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents` | Creates content in a user's channel |
| `content` | `create-s3` | public | v1 | POST | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/s3` | Uploads UGC content to S3 for a user's channel |
| `content` | `delete` | admin | v1 | DELETE | `/ugc/v1/admin/namespaces/{namespace}/channels/{channelId}/contents/{contentId}` | Deletes a content entry from a channel |
| `content` | `delete` | admin | v2 | DELETE | `/ugc/v2/admin/namespaces/{namespace}/channels/{channelId}/contents/{contentId}` | Deletes an official content entry |
| `content` | `delete` | public | v1 | DELETE | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/{contentId}` | Deletes a content entry for a user |
| `content` | `delete` | public | v2 | DELETE | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/{contentId}` | Deletes a content entry |
| `content` | `delete-by-share-code` | admin | v1 | DELETE | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/sharecodes/{shareCode}` | Deletes a content entry by sharecode |
| `content` | `delete-by-share-code` | admin | v2 | DELETE | `/ugc/v2/admin/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/sharecodes/{shareCode}` | Deletes an official content entry by sharecode |
| `content` | `delete-by-share-code` | public | v1 | DELETE | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/sharecodes/{shareCode}` | Deletes a content entry by sharecode |
| `content` | `delete-by-share-code` | public | v2 | DELETE | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/sharecodes/{shareCode}` | Deletes content by sharecode |
| `content` | `delete-screenshot` | admin | v1 | DELETE | `/ugc/v1/admin/namespaces/{namespace}/contents/{contentId}/screenshots/{screenshotId}` | Deletes a screenshot from a content entry |
| `content` | `delete-screenshot` | admin | v2 | DELETE | `/ugc/v2/admin/namespaces/{namespace}/contents/{contentId}/screenshots/{screenshotId}` | Deletes a screenshot from a content entry |
| `content` | `delete-screenshot` | public | v1 | DELETE | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/contents/{contentId}/screenshots/{screenshotId}` | Deletes a screenshot from a content entry |
| `content` | `delete-screenshot` | public | v2 | DELETE | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/contents/{contentId}/screenshots/{screenshotId}` | Deletes a screenshot from a content entry |
| `content` | `delete-user` | admin | v1 | DELETE | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/{contentId}` | Deletes a content entry by ID |
| `content` | `delete-user` | admin | v2 | DELETE | `/ugc/v2/admin/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/{contentId}` | Deletes a user content entry |
| `content` | `generate-file-upload-url` | admin | v1 | POST | `/ugc/v1/admin/namespaces/{namespace}/channels/{channelId}/contents/s3` | Uploads UGC content to S3 for a channel |
| `content` | `generate-screenshot-upload-url` | admin | v1 | POST | `/ugc/v1/admin/namespaces/{namespace}/contents/{contentId}/screenshots` | Uploads screenshots for a content entry |
| `content` | `generate-screenshot-upload-url` | admin | v2 | POST | `/ugc/v2/admin/namespaces/{namespace}/contents/{contentId}/screenshots` | Uploads screenshots for an official content entry |
| `content` | `generate-screenshot-upload-url` | public | v2 | POST | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/contents/{contentId}/screenshots` | Creates screenshot upload URLs for a content entry |
| `content` | `generate-upload-url` | admin | v2 | PATCH | `/ugc/v2/admin/namespaces/{namespace}/channels/{channelId}/contents/{contentId}/uploadUrl` | Generates an upload URL for an official content entry |
| `content` | `generate-upload-url` | public | v1 | POST | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/contents/{contentId}/screenshots` | Uploads screenshots for a content entry |
| `content` | `generate-upload-url` | public | v2 | PATCH | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/{contentId}/uploadUrl` | Generates an upload URL for a content entry |
| `content` | `generate-user-upload-url` | admin | v2 | PATCH | `/ugc/v2/admin/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/{contentId}/uploadUrl` | Generates an upload URL for a user content entry |
| `content` | `get` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/contents/{contentId}` | Retrieves a content entry by ID |
| `content` | `get` | admin | v2 | GET | `/ugc/v2/admin/namespaces/{namespace}/contents/{contentId}` | Retrieves a content entry by ID |
| `content` | `get` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/contents/{contentId}` | Retrieves a content entry by ID |
| `content` | `get` | public | v2 | GET | `/ugc/v2/public/namespaces/{namespace}/contents/{contentId}` | Retrieves a content entry by ID |
| `content` | `get-by-share-code` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/contents/sharecodes/{shareCode}` | Retrieves a content entry by its sharecode |
| `content` | `get-by-share-code` | admin | v2 | GET | `/ugc/v2/admin/namespaces/{namespace}/contents/sharecodes/{shareCode}` | Retrieves content by sharecode |
| `content` | `get-by-share-code` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/contents/sharecodes/{shareCode}` | Retrieves a content entry by its sharecode |
| `content` | `get-by-share-code` | public | v2 | GET | `/ugc/v2/public/namespaces/{namespace}/contents/sharecodes/{shareCode}` | Retrieves content by sharecode |
| `content` | `get-preview` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/contents/{contentId}/preview` | Retrieves the preview for a content entry |
| `content` | `get-preview` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/contents/{contentId}/preview` | Retrieves the preview for a content entry |
| `content` | `get-user` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/contents` | Lists content created by a user |
| `content` | `get-user` | public | v2 | GET | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/contents` | Lists content created by a user |
| `content` | `list` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/contents` | Lists user-generated contents in a namespace |
| `content` | `list` | admin | v2 | GET | `/ugc/v2/admin/namespaces/{namespace}/contents` | Lists official content in the namespace |
| `content` | `list` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/contents` | Searches contents in a namespace |
| `content` | `list` | public | v2 | GET | `/ugc/v2/public/namespaces/{namespace}/contents` | Lists public content in the namespace |
| `content` | `list-channel` | admin | v2 | GET | `/ugc/v2/admin/namespaces/{namespace}/channels/{channelId}/contents` | Lists official content in a channel |
| `content` | `list-channel` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/channels/{channelId}/contents` | Searches contents within a channel |
| `content` | `list-channel` | public | v2 | GET | `/ugc/v2/public/namespaces/{namespace}/channels/{channelId}/contents` | Lists content in a channel |
| `content` | `list-user` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/contents` | Lists user-generated contents for a specific user |
| `content` | `list-user` | admin | v2 | GET | `/ugc/v2/admin/namespaces/{namespace}/users/{userId}/contents` | Lists content created by a user |
| `content` | `list-versions` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/contents/{contentId}/versions` | Lists file versions for a content entry |
| `content` | `list-versions` | admin | v2 | GET | `/ugc/v2/admin/namespaces/{namespace}/contents/{contentId}/versions` | Lists file versions for a content entry |
| `content` | `rollback` | admin | v1 | PUT | `/ugc/v1/admin/namespaces/{namespace}/contents/{contentId}/rollback/{versionId}` | Restores a content entry to a previous file version |
| `content` | `rollback` | admin | v2 | PUT | `/ugc/v2/admin/namespaces/{namespace}/contents/{contentId}/rollback/{versionId}` | Restores a content entry to a specific file version |
| `content` | `search` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/contents/search` | Searches contents in a namespace |
| `content` | `search-channel` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/channels/{channelId}/contents/search` | Searches contents within a channel |
| `content` | `update` | admin | v2 | PATCH | `/ugc/v2/admin/namespaces/{namespace}/channels/{channelId}/contents/{contentId}` | Updates an official content entry |
| `content` | `update` | public | v2 | PATCH | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/{contentId}` | Updates a content entry |
| `content` | `update-by-share-code` | admin | v2 | PUT | `/ugc/v2/admin/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/s3/sharecodes/{shareCode}` | Updates official content in S3 by sharecode |
| `content` | `update-by-share-code` | public | v1 | PUT | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/s3/sharecodes/{shareCode}` | Updates UGC content in S3 by sharecode |
| `content` | `update-by-share-code` | public | v2 | PUT | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/s3/sharecodes/{shareCode}` | Updates UGC content in S3 by sharecode |
| `content` | `update-file` | admin | v1 | PUT | `/ugc/v1/admin/namespaces/{namespace}/channels/{channelId}/contents/s3/{contentId}` | Updates UGC content in S3 for a channel |
| `content` | `update-file` | public | v1 | PUT | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/s3/{contentId}` | Updates UGC content in S3 for a user's channel |
| `content` | `update-file-by-share-code` | admin | v1 | PUT | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/s3/sharecodes/{shareCode}` | Updates UGC content in S3 by sharecode |
| `content` | `update-file-location` | admin | v2 | PATCH | `/ugc/v2/admin/namespaces/{namespace}/channels/{channelId}/contents/{contentId}/fileLocation` | Updates the file location for an official content entry |
| `content` | `update-file-location` | public | v2 | PATCH | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/{contentId}/fileLocation` | Updates the file location for a content entry |
| `content` | `update-file-location-user` | admin | v2 | PATCH | `/ugc/v2/admin/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/{contentId}/fileLocation` | Updates the file location for a user content entry |
| `content` | `update-file-user` | admin | v1 | PUT | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/s3/{contentId}` | Updates UGC content in S3 for a user's channel |
| `content` | `update-screenshots` | admin | v1 | PUT | `/ugc/v1/admin/namespaces/{namespace}/contents/{contentId}/screenshots` | Updates screenshots for a content entry |
| `content` | `update-screenshots` | admin | v2 | PUT | `/ugc/v2/admin/namespaces/{namespace}/contents/{contentId}/screenshots` | Updates screenshots for an official content entry |
| `content` | `update-screenshots` | public | v1 | PUT | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/contents/{contentId}/screenshots` | Updates screenshots for a content entry |
| `content` | `update-screenshots` | public | v2 | PUT | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/contents/{contentId}/screenshots` | Updates screenshots for a content entry |
| `content` | `update-share-code` | public | v1 | PATCH | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/{contentId}/sharecode` | Updates the sharecode for a content entry |
| `content` | `update-share-code` | public | v2 | PATCH | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/{contentId}/sharecode` | Updates the sharecode for a content entry |
| `content` | `update-user` | admin | v2 | PATCH | `/ugc/v2/admin/namespaces/{namespace}/users/{userId}/channels/{channelId}/contents/{contentId}` | Updates a user content entry |
| `content` | `update-visibility` | admin | v1 | PUT | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/contents/{contentId}/hide` | Updates the hidden status of a user's content |
| `content` | `update-visibility` | admin | v2 | PUT | `/ugc/v2/admin/namespaces/{namespace}/users/{userId}/contents/{contentId}/hide` | Sets the visibility of a user content entry |
| `creators` | `get` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/users/{userId}` | Retrieves stats for a creator including likes, followers, and following counts |
| `creators` | `search` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/users` | Searches for creators in a namespace |
| `engagements` | `follow` | public | v1 | PUT | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/follow` | Updates the follow status for a user |
| `engagements` | `like` | public | v1 | PUT | `/ugc/v1/public/namespaces/{namespace}/contents/{contentId}/like` | Updates the like status for a content entry |
| `engagements` | `like` | public | v2 | PUT | `/ugc/v2/public/namespaces/{namespace}/contents/{contentId}/like` | Updates the like status for a content entry |
| `engagements` | `list-content-followed` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/contents/followed` | Lists contents from followed creators |
| `engagements` | `list-creators-followed` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/users/followed` | Lists followed creators |
| `engagements` | `list-downloaders` | public | v2 | GET | `/ugc/v2/public/namespaces/{namespace}/contents/{contentId}/downloader` | Lists users who downloaded a content entry |
| `engagements` | `list-followed` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/following` | Lists users followed by a user |
| `engagements` | `list-followers` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/followers` | Lists followers of a user |
| `engagements` | `list-liked` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/contents/liked` | Lists liked contents |
| `engagements` | `list-likers` | public | v2 | GET | `/ugc/v2/public/namespaces/{namespace}/contents/{contentId}/like` | Lists users who have liked a content entry |
| `engagements` | `set-download-count` | public | v1 | POST | `/ugc/v1/public/namespaces/{namespace}/contents/{contentId}/downloadcount` | Adds a unique download count to a content entry |
| `engagements` | `set-download-count` | public | v2 | POST | `/ugc/v2/public/namespaces/{namespace}/contents/{contentId}/downloadcount` | Adds a download count to a content entry |
| `groups` | `create` | admin | v1 | POST | `/ugc/v1/admin/namespaces/{namespace}/groups` | Creates a group in the namespace |
| `groups` | `create` | public | v1 | POST | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/groups` | Creates a content group for a user |
| `groups` | `delete` | admin | v1 | DELETE | `/ugc/v1/admin/namespaces/{namespace}/groups/{groupId}` | Deletes a group by ID |
| `groups` | `delete` | public | v1 | DELETE | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/groups/{groupId}` | Deletes a content group for a user |
| `groups` | `delete-user` | admin | v1 | DELETE | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/groups/{groupId}` | Deletes a group for a user |
| `groups` | `get` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/groups/{groupId}` | Retrieves a specific user group by ID |
| `groups` | `get` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/groups/{groupId}` | Retrieves a content group for a user |
| `groups` | `get-user` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/groups/{groupId}` | Retrieves a specific group for a user |
| `groups` | `list` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/groups` | Lists all user groups in a namespace |
| `groups` | `list` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/groups` | Lists all groups for a user |
| `groups` | `list-content` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/groups/{groupId}/contents` | Lists contents belonging to a group (legacy) |
| `groups` | `list-content` | admin | v2 | GET | `/ugc/v2/admin/namespaces/{namespace}/groups/{groupId}/contents` | Lists content in a group |
| `groups` | `list-content` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/groups/{groupId}/contents` | Lists content in a group (legacy) |
| `groups` | `list-content` | public | v2 | GET | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/groups/{groupId}/contents` | Lists content entries belonging to a group |
| `groups` | `list-content-user` | admin | v2 | GET | `/ugc/v2/admin/namespaces/{namespace}/users/{userId}/groups/{groupId}/contents` | Lists content in a group for a user |
| `groups` | `list-contents-user` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/groups/{groupId}/contents` | Lists contents belonging to a group for a user (legacy) |
| `groups` | `list-user` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/groups` | Lists all groups for a user |
| `groups` | `update` | admin | v1 | PUT | `/ugc/v1/admin/namespaces/{namespace}/groups/{groupId}` | Updates a group by replacing its name and contents |
| `groups` | `update` | public | v1 | PUT | `/ugc/v1/public/namespaces/{namespace}/users/{userId}/groups/{groupId}` | Updates a content group for a user |
| `groups` | `update-user` | admin | v1 | PUT | `/ugc/v1/admin/namespaces/{namespace}/users/{userId}/groups/{groupId}` | Updates a group for a user |
| `staging` | `delete` | public | v2 | DELETE | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/staging-contents/{contentId}` | Deletes a user staging content entry by ID |
| `staging` | `get` | public | v2 | GET | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/staging-contents/{contentId}` | Retrieves a staging content entry by ID |
| `staging` | `list` | public | v2 | GET | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/staging-contents` | Lists staging content for a user |
| `staging` | `update` | public | v2 | PUT | `/ugc/v2/public/namespaces/{namespace}/users/{userId}/staging-contents/{contentId}` | Updates a staging content entry |
| `staging-content` | `approve` | admin | v2 | POST | `/ugc/v2/admin/namespaces/{namespace}/staging-contents/{contentId}/approve` | Approves or rejects a staging content entry |
| `staging-content` | `get` | admin | v2 | GET | `/ugc/v2/admin/namespaces/{namespace}/staging-contents/{contentId}` | Retrieves a staging content entry by ID |
| `staging-content` | `list` | admin | v2 | GET | `/ugc/v2/admin/namespaces/{namespace}/staging-contents` | Lists staging content pending approval |
| `staging-content` | `list-user` | admin | v2 | GET | `/ugc/v2/admin/namespaces/{namespace}/users/{userId}/staging-contents` | Lists staging content for a user pending approval |
| `tags` | `create` | admin | v1 | POST | `/ugc/v1/admin/namespaces/{namespace}/tags` | Creates a tag in the namespace |
| `tags` | `delete` | admin | v1 | DELETE | `/ugc/v1/admin/namespaces/{namespace}/tags/{tagId}` | Deletes a tag by ID |
| `tags` | `list` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/tags` | Lists available tags in a namespace |
| `tags` | `list` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/tags` | Lists available tags in a namespace |
| `tags` | `update` | admin | v1 | PUT | `/ugc/v1/admin/namespaces/{namespace}/tags/{tagId}` | Updates an existing tag |
| `types` | `create` | admin | v1 | POST | `/ugc/v1/admin/namespaces/{namespace}/types` | Creates a type and subtype in the namespace |
| `types` | `delete` | admin | v1 | DELETE | `/ugc/v1/admin/namespaces/{namespace}/types/{typeId}` | Deletes a type by ID |
| `types` | `list` | admin | v1 | GET | `/ugc/v1/admin/namespaces/{namespace}/types` | Lists available content types in a namespace |
| `types` | `list` | public | v1 | GET | `/ugc/v1/public/namespaces/{namespace}/types` | Lists available content types in a namespace |
| `types` | `update` | admin | v1 | PUT | `/ugc/v1/admin/namespaces/{namespace}/types/{typeId}` | Updates a type and its subtypes |

