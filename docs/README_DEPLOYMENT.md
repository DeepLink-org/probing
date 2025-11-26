# Documentation Deployment Guide

This guide explains how the documentation is automatically built and deployed.

## Automated Deployment

The documentation is automatically built and deployed using GitHub Actions when:
- Changes are pushed to `master` or `main` branch in the `docs/` directory
- Pull requests are opened that modify files in `docs/`
- The workflow is manually triggered via `workflow_dispatch`

## Deployment Targets

### 1. GitHub Pages

The documentation is automatically deployed to GitHub Pages when changes are pushed to the main branch. The built HTML files are published to the `gh-pages` branch.

**Access**: `https://<username>.github.io/<repository>/` (if configured) or via repository settings.

### 2. Read the Docs

Read the Docs can be configured to automatically build from the repository using webhooks, or the workflow can trigger builds via the Read the Docs API.

#### Option A: Webhook (Recommended)

1. Go to your Read the Docs project settings
2. Enable "Build pull requests" if desired
3. Configure the webhook in GitHub repository settings:
   - Go to Settings → Webhooks
   - Add webhook: `https://readthedocs.org/api/v2/webhook/<project-slug>/<id>/`
   - Select "Just the push event"

#### Option B: API Token (Alternative)

If you prefer to trigger builds via API from the workflow:

1. Create a Read the Docs API token:
   - Go to https://readthedocs.org/accounts/token/
   - Create a new token with appropriate permissions

2. Add GitHub secrets:
   - `RTD_API_TOKEN`: Your Read the Docs API token
   - `RTD_PROJECT_SLUG`: Your project slug (defaults to 'probing' if not set)

3. The workflow will automatically trigger builds when documentation changes are pushed to the main branch.

## Local Testing

Before pushing changes, you can test the documentation build locally:

```bash
cd docs
pip install -r requirements_doc.txt
make html
```

To preview the documentation:

```bash
make serve
# Or use the serve.sh script
./serve.sh
```

## Workflow Details

The workflow (`.github/workflows/docs.yml`) performs the following steps:

1. **Checkout** the repository
2. **Set up Python** 3.12 environment
3. **Install** documentation dependencies from `requirements_doc.txt`
4. **Build** the documentation using Sphinx
5. **Check links** (with warnings allowed)
6. **Upload artifacts** for review
7. **Deploy to GitHub Pages** (main branch only)
8. **Trigger Read the Docs build** via API (if token is configured)

## Troubleshooting

### Build Failures

- Check the GitHub Actions logs for specific error messages
- Verify that all dependencies in `requirements_doc.txt` are up to date
- Test the build locally before pushing

### Read the Docs Not Building

- Verify webhook is configured correctly
- Check Read the Docs project settings for build configuration
- If using API token, verify the token has correct permissions
- Check the workflow logs for API call responses

### GitHub Pages Not Updating

- Verify GitHub Pages is enabled in repository settings
- Check that the workflow has permission to write to the repository
- Review the workflow logs for deployment errors

